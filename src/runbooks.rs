use crate::alert::{tokenize, Alert};
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

const EXCERPT_CHARS: usize = 900;

#[derive(Debug, Clone, Serialize)]
pub struct Runbook {
    pub path: String,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoredRunbook {
    pub path: String,
    pub title: String,
    pub score: f64,
    pub excerpt: String,
}

/// Load every `*.md` file in the runbook directory. Title = first `#` heading.
pub fn load_runbooks(dir: &Path) -> Result<Vec<Runbook>> {
    let mut runbooks = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading runbook directory {}", dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "md") {
            let content = std::fs::read_to_string(&path)?;
            let title = content
                .lines()
                .find(|l| l.starts_with('#'))
                .map(|l| l.trim_start_matches('#').trim().to_string())
                .unwrap_or_else(|| {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            runbooks.push(Runbook {
                path: path.to_string_lossy().into_owned(),
                title,
                content,
            });
        }
    }
    runbooks.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(runbooks)
}

/// Cheap keyword-overlap pre-ranking; the LLM makes the final pick from the top candidates.
pub fn rank(alert: &Alert, runbooks: &[Runbook], top_n: usize) -> Vec<ScoredRunbook> {
    let alert_tokens = tokenize(&alert.search_text());
    let mut scored: Vec<ScoredRunbook> = runbooks
        .iter()
        .map(|rb| {
            let title_tokens = tokenize(&rb.title);
            let body_tokens = tokenize(&rb.content);
            let title_hits = alert_tokens
                .iter()
                .filter(|t| title_tokens.contains(t))
                .count();
            let body_hits = alert_tokens
                .iter()
                .filter(|t| body_tokens.contains(t))
                .count();
            ScoredRunbook {
                path: rb.path.clone(),
                title: rb.title.clone(),
                score: title_hits as f64 * 3.0 + body_hits as f64,
                excerpt: rb.content.chars().take(EXCERPT_CHARS).collect(),
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_n);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn runbook(path: &str, title: &str, content: &str) -> Runbook {
        Runbook {
            path: path.into(),
            title: title.into(),
            content: content.into(),
        }
    }

    #[test]
    fn db_alert_ranks_db_runbook_first() {
        let alert: Alert = serde_json::from_value(json!({
            "name": "DBConnPoolExhausted",
            "service": "orders-db",
            "summary": "database connection pool saturated"
        }))
        .unwrap();
        let runbooks = vec![
            runbook("rb/latency.md", "High Latency", "p95 latency spikes, check CDN and cache hit rates"),
            runbook(
                "rb/db.md",
                "Database Connection Pool Exhaustion",
                "When the connection pool is saturated:\n1. Check pool metrics\n2. Restart leaking pods",
            ),
        ];
        let ranked = rank(&alert, &runbooks, 2);
        assert_eq!(ranked[0].path, "rb/db.md");
        assert!(ranked[0].score > ranked[1].score);
    }
}
