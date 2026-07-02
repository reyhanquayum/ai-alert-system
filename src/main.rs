mod alert;
mod commits;
mod config;
mod impact;
mod incident;
mod llm;
mod pipeline;
mod postmortem;
mod runbooks;
mod slack;

use alert::Alert;
use anyhow::{anyhow, Context, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Parser, Subcommand};
use config::Config;
use pipeline::Pipeline;
use serde_json::{json, Value};
use tracing::{error, info};

const SAMPLE_ALERTS: &[(&str, &str)] = &[
    (
        "high-error-rate",
        include_str!("../examples/alerts/high-error-rate.json"),
    ),
    (
        "db-connections",
        include_str!("../examples/alerts/db-connections.json"),
    ),
    (
        "high-latency",
        include_str!("../examples/alerts/high-latency.json"),
    ),
];

#[derive(Parser)]
#[command(name = "ai-alert-system", about = "Autonomous AI incident responder")]
struct Cli {
    /// Path to the TOML config file (defaults are used if it doesn't exist)
    #[arg(long, global = true, default_value = "config.toml")]
    config: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the webhook server (POST /alerts, POST /incidents/{id}/resolve, ...)
    Serve,
    /// Fire a sample (or custom) alert through the full pipeline, no server needed
    Simulate {
        /// One of the built-in samples: high-error-rate | db-connections | high-latency
        #[arg(long, default_value = "high-error-rate")]
        sample: String,
        /// Path to a custom alert JSON file (overrides --sample)
        #[arg(long)]
        file: Option<String>,
        /// Git repo to analyze for the suspect commit (overrides config)
        #[arg(long)]
        repo: Option<String>,
        /// Also resolve the incident with this note, generating the postmortem
        #[arg(long)]
        resolve: Option<String>,
    },
    /// Resolve an incident and generate its postmortem
    Resolve {
        id: String,
        #[arg(long)]
        note: String,
    },
    /// List all incidents
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let mut cfg = Config::load(&cli.config)?;

    match cli.cmd {
        Cmd::Serve => {
            let pipeline = Pipeline::new(cfg.clone())?;
            serve(cfg, pipeline).await
        }
        Cmd::Simulate {
            sample,
            file,
            repo,
            resolve,
        } => {
            if let Some(repo) = repo {
                cfg.repo.path = repo;
            }
            let pipeline = Pipeline::new(cfg)?;
            simulate(pipeline, &sample, file.as_deref(), resolve.as_deref()).await
        }
        Cmd::Resolve { id, note } => {
            let pipeline = Pipeline::new(cfg)?;
            let incident = pipeline.resolve(&id, &note).await?;
            println!(
                "resolved {} — postmortem: {}",
                incident.id,
                incident
                    .postmortem_path
                    .as_deref()
                    .unwrap_or("(generation failed)")
            );
            Ok(())
        }
        Cmd::List => {
            let pipeline = Pipeline::new(cfg)?;
            for i in pipeline.store.list()? {
                println!(
                    "{}  {:12}  {:8}  {}",
                    i.id,
                    format!("{:?}", i.status).to_lowercase(),
                    i.alert.severity,
                    i.alert.name
                );
            }
            Ok(())
        }
    }
}

async fn simulate(
    pipeline: Pipeline,
    sample: &str,
    file: Option<&str>,
    resolve_note: Option<&str>,
) -> Result<()> {
    let raw = match file {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
        None => SAMPLE_ALERTS
            .iter()
            .find(|(name, _)| *name == sample)
            .map(|(_, body)| body.to_string())
            .ok_or_else(|| {
                anyhow!(
                    "unknown sample '{sample}' (available: {})",
                    SAMPLE_ALERTS
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?,
    };
    let payload: Value = serde_json::from_str(&raw)?;
    let alerts = Alert::from_payload(&payload)?;

    for alert in alerts {
        println!("\n=== alert: {} ({}) ===", alert.name, alert.severity);
        let incident = pipeline.handle_alert(alert).await?;
        println!("\n--- incident {} ---", incident.id);
        if let Some(brief) = &incident.brief {
            println!("\n[slack brief]\n{brief}");
        }
        if let Some(note) = resolve_note {
            let resolved = pipeline.resolve(&incident.id, note).await?;
            if let Some(path) = &resolved.postmortem_path {
                println!("\n[postmortem written to {path}]");
                println!("{}", std::fs::read_to_string(path)?);
            }
        } else {
            println!(
                "\nresolve later with: cargo run -- resolve {} --note \"what fixed it\"",
                incident.id
            );
        }
    }
    Ok(())
}

#[derive(Clone)]
struct AppState {
    pipeline: Pipeline,
}

async fn serve(cfg: Config, pipeline: Pipeline) -> Result<()> {
    let state = AppState { pipeline };
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/alerts", post(post_alerts))
        .route("/incidents", get(list_incidents))
        .route("/incidents/{id}", get(get_incident))
        .route("/incidents/{id}/resolve", post(resolve_incident))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.server.bind)
        .await
        .with_context(|| format!("binding {}", cfg.server.bind))?;
    info!("listening on http://{}", cfg.server.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

type ApiError = (StatusCode, String);

fn bad_request(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, e.to_string())
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Ingest alert(s). Incidents are created synchronously (so IDs can be
/// returned) and investigation continues in the background.
async fn post_alerts(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let alerts = Alert::from_payload(&payload).map_err(bad_request)?;
    let mut ids = Vec::new();
    for alert in alerts {
        let mut incident = state.pipeline.open_incident(alert).map_err(internal)?;
        ids.push(incident.id.clone());
        let pipeline = state.pipeline.clone();
        tokio::spawn(async move {
            pipeline.investigate(&mut incident).await;
        });
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "incidents": ids }))))
}

async fn list_incidents(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let incidents = state.pipeline.store.list().map_err(internal)?;
    let summary: Vec<Value> = incidents
        .iter()
        .map(|i| {
            json!({
                "id": i.id,
                "status": i.status,
                "alert": i.alert.name,
                "severity": i.alert.severity,
                "created_at": i.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "incidents": summary })))
}

async fn get_incident(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let incident = state
        .pipeline
        .store
        .load(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::to_value(incident).map_err(internal)?))
}

async fn resolve_incident(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let note = body
        .get("note")
        .and_then(|n| n.as_str())
        .ok_or_else(|| bad_request("body must be {\"note\": \"...\"}"))?;
    let incident = state.pipeline.resolve(&id, note).await.map_err(|e| {
        error!("resolve failed: {e:#}");
        internal(e)
    })?;
    Ok(Json(serde_json::to_value(incident).map_err(internal)?))
}
