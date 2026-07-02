use crate::alert::Alert;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Investigating,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub at: DateTime<Utc>,
    pub event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspectCommit {
    pub hash: String,
    pub author: String,
    pub message: String,
    pub confidence: String,
    pub reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookMatch {
    pub path: String,
    pub title: String,
    pub reasoning: String,
    pub key_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactEstimate {
    pub severity_assessment: String,
    pub affected_users: String,
    pub narrative: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub status: IncidentStatus,
    pub alert: Alert,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub suspect_commit: Option<SuspectCommit>,
    pub runbook: Option<RunbookMatch>,
    pub impact: Option<ImpactEstimate>,
    pub brief: Option<String>,
    pub resolution: Option<String>,
    pub postmortem_path: Option<String>,
    pub timeline: Vec<TimelineEntry>,
}

impl Incident {
    pub fn new(alert: Alert) -> Self {
        let now = Utc::now();
        let id = format!(
            "INC-{}-{}",
            now.format("%Y%m%d%H%M%S"),
            &uuid::Uuid::new_v4().simple().to_string()[..4]
        );
        let mut incident = Incident {
            id,
            status: IncidentStatus::Investigating,
            alert,
            created_at: now,
            resolved_at: None,
            suspect_commit: None,
            runbook: None,
            impact: None,
            brief: None,
            resolution: None,
            postmortem_path: None,
            timeline: Vec::new(),
        };
        incident.log("alert received; incident opened");
        incident
    }

    pub fn log(&mut self, event: impl Into<String>) {
        self.timeline.push(TimelineEntry { at: Utc::now(), event: event.into() });
    }
}

/// JSON-file-per-incident store. Deliberately simple; swap for a DB later.
#[derive(Debug, Clone)]
pub struct IncidentStore {
    dir: PathBuf,
}

impl IncidentStore {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn save(&self, incident: &Incident) -> Result<()> {
        let path = self.path_for(&incident.id);
        let json = serde_json::to_string_pretty(incident)?;
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Incident> {
        let path = self.path_for(id);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("incident {id} not found at {}", path.display()))?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn list(&self) -> Result<Vec<Incident>> {
        let mut incidents = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Ok(incident) = serde_json::from_str::<Incident>(&raw) {
                        incidents.push(incident);
                    }
                }
            }
        }
        incidents.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(incidents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = IncidentStore::new(dir.path()).unwrap();
        let alert: Alert = serde_json::from_value(json!({"name": "TestAlert"})).unwrap();
        let incident = Incident::new(alert);
        store.save(&incident).unwrap();
        let loaded = store.load(&incident.id).unwrap();
        assert_eq!(loaded.id, incident.id);
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
