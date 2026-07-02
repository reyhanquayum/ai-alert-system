use crate::alert::Alert;
use crate::config::Config;
use crate::incident::{
    ImpactEstimate, Incident, IncidentStatus, IncidentStore, RunbookMatch, SuspectCommit,
};
use crate::llm::{Llm, Task};
use crate::{commits, impact, postmortem, runbooks, slack};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Orchestrates the whole incident lifecycle. Cheap to clone (Arc fields).
#[derive(Clone)]
pub struct Pipeline {
    pub cfg: Arc<Config>,
    pub llm: Arc<Llm>,
    pub store: Arc<IncidentStore>,
    http: reqwest::Client,
}

impl Pipeline {
    pub fn new(cfg: Config) -> Result<Self> {
        let llm = if cfg.llm_use_real() {
            let key =
                cfg.llm.api_key.clone().ok_or_else(|| {
                    anyhow!("llm mode is 'real' but ANTHROPIC_API_KEY is not set")
                })?;
            info!(model = %cfg.llm.model, "LLM backend: Claude API");
            Llm::real(key, cfg.llm.model.clone(), cfg.llm.max_tokens)
        } else {
            info!("LLM backend: mock (set ANTHROPIC_API_KEY to use the Claude API)");
            Llm::mock()
        };
        let store = IncidentStore::new(&cfg.storage.incidents_dir)?;
        Ok(Self {
            cfg: Arc::new(cfg),
            llm: Arc::new(llm),
            store: Arc::new(store),
            http: reqwest::Client::new(),
        })
    }

    /// Create + persist a new incident for an alert (fast; called inline by the webhook).
    pub fn open_incident(&self, alert: Alert) -> Result<Incident> {
        let incident = Incident::new(alert);
        self.store.save(&incident)?;
        info!(id = %incident.id, alert = %incident.alert.name, "incident opened");
        Ok(incident)
    }

    /// Full alert handling: open, investigate, brief. Returns the final record.
    pub async fn handle_alert(&self, alert: Alert) -> Result<Incident> {
        let mut incident = self.open_incident(alert)?;
        self.investigate(&mut incident).await;
        Ok(incident)
    }

    /// Run the three analyses concurrently, compose the brief, post to Slack.
    /// Individual analysis failures degrade gracefully (logged, brief still goes out).
    pub async fn investigate(&self, incident: &mut Incident) {
        let (suspect, runbook, impact_est) = tokio::join!(
            self.find_suspect_commit(&incident.alert),
            self.match_runbook(&incident.alert),
            self.estimate_impact(&incident.alert),
        );

        match suspect {
            Ok(Some(sc)) => {
                incident.log(format!(
                    "suspect commit identified: {} ({})",
                    sc.hash, sc.confidence
                ));
                incident.suspect_commit = Some(sc);
            }
            Ok(None) => incident.log("no plausible suspect commit found"),
            Err(e) => {
                warn!(id = %incident.id, "suspect-commit analysis failed: {e:#}");
                incident.log(format!("suspect-commit analysis failed: {e}"));
            }
        }
        match runbook {
            Ok(rb) => {
                incident.log(format!("runbook matched: {}", rb.path));
                incident.runbook = Some(rb);
            }
            Err(e) => {
                warn!(id = %incident.id, "runbook matching failed: {e:#}");
                incident.log(format!("runbook matching failed: {e}"));
            }
        }
        match impact_est {
            Ok(imp) => {
                incident.log("impact estimated");
                incident.impact = Some(imp);
            }
            Err(e) => {
                warn!(id = %incident.id, "impact estimation failed: {e:#}");
                incident.log(format!("impact estimation failed: {e}"));
            }
        }

        match self
            .llm
            .generate(Task::ComposeBrief, &json!({ "incident": incident }))
            .await
        {
            Ok(brief) => {
                if let Err(e) =
                    slack::post(&self.http, self.cfg.slack.webhook_url.as_deref(), &brief).await
                {
                    error!(id = %incident.id, "Slack post failed: {e:#}");
                    incident.log(format!("Slack post failed: {e}"));
                } else {
                    incident.log("brief posted to Slack");
                }
                incident.brief = Some(brief);
            }
            Err(e) => {
                error!(id = %incident.id, "brief composition failed: {e:#}");
                incident.log(format!("brief composition failed: {e}"));
            }
        }

        if let Err(e) = self.store.save(incident) {
            error!(id = %incident.id, "failed to persist incident: {e:#}");
        }
    }

    async fn find_suspect_commit(&self, alert: &Alert) -> Result<Option<SuspectCommit>> {
        let repo = Path::new(&self.cfg.repo.path);
        let commits = commits::recent_commits(
            repo,
            self.cfg.repo.lookback_hours,
            self.cfg.repo.max_commits,
        )
        .await?;
        if commits.is_empty() {
            return Ok(None);
        }
        let ctx = json!({ "alert": alert, "commits": commits });
        let verdict = self.llm.analyze(Task::SuspectCommit, &ctx).await?;
        let Some(hash) = verdict["hash"].as_str().filter(|h| !h.is_empty()) else {
            return Ok(None);
        };
        let commit = commits
            .iter()
            .find(|c| c.hash.starts_with(hash) || hash.starts_with(&c.hash))
            .ok_or_else(|| anyhow!("model chose unknown commit hash {hash}"))?;
        Ok(Some(SuspectCommit {
            hash: commit.hash.clone(),
            author: commit.author.clone(),
            message: commit.message.clone(),
            confidence: verdict["confidence"].as_str().unwrap_or("low").to_string(),
            reasoning: verdict["reasoning"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        }))
    }

    async fn match_runbook(&self, alert: &Alert) -> Result<RunbookMatch> {
        let dir = Path::new(&self.cfg.runbooks.dir);
        let all = runbooks::load_runbooks(dir)?;
        if all.is_empty() {
            return Err(anyhow!("no runbooks found in {}", dir.display()));
        }
        let candidates = runbooks::rank(alert, &all, 5);
        let ctx = json!({ "alert": alert, "candidates": candidates });
        let verdict = self.llm.analyze(Task::SelectRunbook, &ctx).await?;
        let path = verdict["path"]
            .as_str()
            .ok_or_else(|| anyhow!("model reply missing runbook path"))?;
        let chosen = candidates
            .iter()
            .find(|c| c.path == path)
            .ok_or_else(|| anyhow!("model chose unknown runbook {path}"))?;
        let key_steps = verdict["key_steps"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(RunbookMatch {
            path: chosen.path.clone(),
            title: chosen.title.clone(),
            reasoning: verdict["reasoning"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            key_steps,
        })
    }

    async fn estimate_impact(&self, alert: &Alert) -> Result<ImpactEstimate> {
        let metrics = impact::snapshot_for(alert);
        let ctx = json!({ "alert": alert, "metrics": metrics });
        let verdict = self.llm.analyze(Task::EstimateImpact, &ctx).await?;
        Ok(ImpactEstimate {
            severity_assessment: verdict["severity_assessment"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            affected_users: verdict["affected_users"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            narrative: verdict["narrative"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        })
    }

    /// Resolve an incident: record the note, generate the postmortem, notify Slack.
    pub async fn resolve(&self, id: &str, note: &str) -> Result<Incident> {
        let mut incident = self.store.load(id)?;
        if incident.status == IncidentStatus::Resolved {
            return Err(anyhow!("incident {id} is already resolved"));
        }
        incident.status = IncidentStatus::Resolved;
        incident.resolved_at = Some(Utc::now());
        incident.resolution = Some(note.to_string());
        incident.log(format!("incident resolved: {note}"));

        let pm_dir = Path::new(&self.cfg.storage.postmortems_dir);
        match postmortem::generate_and_save(&self.llm, &incident, pm_dir).await {
            Ok(path) => {
                incident.log(format!("postmortem generated: {}", path.display()));
                incident.postmortem_path = Some(path.to_string_lossy().into_owned());
            }
            Err(e) => {
                error!(id = %incident.id, "postmortem generation failed: {e:#}");
                incident.log(format!("postmortem generation failed: {e}"));
            }
        }

        let msg = format!(
            ":white_check_mark: *{} resolved* — {}\nPostmortem: `{}`",
            incident.id,
            note,
            incident
                .postmortem_path
                .as_deref()
                .unwrap_or("(generation failed)")
        );
        if let Err(e) = slack::post(&self.http, self.cfg.slack.webhook_url.as_deref(), &msg).await {
            warn!(id = %incident.id, "Slack resolution post failed: {e:#}");
        }

        self.store.save(&incident)?;
        info!(id = %incident.id, "incident resolved");
        Ok(incident)
    }
}
