use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Normalized alert, regardless of which monitoring system produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub name: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default = "default_service")]
    pub service: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default = "Utc::now")]
    pub started_at: DateTime<Utc>,
}

fn default_severity() -> String {
    "warning".into()
}
fn default_service() -> String {
    "unknown".into()
}

impl Alert {
    /// Parse an incoming webhook payload. Supports:
    /// - Prometheus Alertmanager: `{"alerts": [{"labels": {...}, "annotations": {...}, "startsAt": ...}]}`
    /// - Generic: a single object (or array of objects) matching `Alert`'s own shape.
    pub fn from_payload(payload: &Value) -> Result<Vec<Alert>> {
        if let Some(alerts) = payload.get("alerts").and_then(|a| a.as_array()) {
            let parsed: Vec<Alert> = alerts.iter().map(Self::from_alertmanager).collect::<Result<_>>()?;
            if parsed.is_empty() {
                bail!("alertmanager payload contained no alerts");
            }
            return Ok(parsed);
        }
        if let Some(arr) = payload.as_array() {
            return arr
                .iter()
                .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
                .collect();
        }
        Ok(vec![serde_json::from_value(payload.clone())?])
    }

    fn from_alertmanager(item: &Value) -> Result<Alert> {
        let labels: BTreeMap<String, String> = item
            .get("labels")
            .and_then(|l| serde_json::from_value(l.clone()).ok())
            .unwrap_or_default();
        let get_ann = |key: &str| -> String {
            item.get("annotations")
                .and_then(|a| a.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let name = labels
            .get("alertname")
            .cloned()
            .unwrap_or_else(|| "UnnamedAlert".into());
        let severity = labels.get("severity").cloned().unwrap_or_else(default_severity);
        let service = labels
            .get("service")
            .or_else(|| labels.get("job"))
            .cloned()
            .unwrap_or_else(default_service);
        let started_at = item
            .get("startsAt")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(Alert {
            name,
            severity,
            service,
            summary: get_ann("summary"),
            description: get_ann("description"),
            labels,
            started_at,
        })
    }

    /// Free-text used for keyword matching (runbooks, commits).
    pub fn search_text(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.name,
            self.service,
            self.summary,
            self.description,
            self.labels
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

/// Lowercased alphanumeric tokens, for cheap keyword-overlap scoring.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            cur.extend(ch.to_lowercase());
        } else if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    // Also split camelCase-ish alert names like "CheckoutHighErrorRate".
    let mut extra = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let mut word = String::new();
        for ch in raw.chars() {
            if ch.is_uppercase() && !word.is_empty() {
                extra.push(word.to_lowercase());
                word = String::new();
            }
            word.push(ch);
        }
        if !word.is_empty() {
            extra.push(word.to_lowercase());
        }
    }
    tokens.extend(extra);
    tokens.sort();
    tokens.dedup();
    tokens.retain(|t| t.len() > 2);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_generic_alert() {
        let payload = json!({
            "name": "CheckoutHighErrorRate",
            "severity": "critical",
            "service": "checkout-api",
            "summary": "5xx error rate above 20%"
        });
        let alerts = Alert::from_payload(&payload).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].name, "CheckoutHighErrorRate");
        assert_eq!(alerts[0].severity, "critical");
    }

    #[test]
    fn parses_alertmanager_payload() {
        let payload = json!({
            "receiver": "ai-alert-system",
            "alerts": [{
                "labels": {"alertname": "DBConnPoolExhausted", "severity": "critical", "service": "orders-db"},
                "annotations": {"summary": "connection pool saturated"},
                "startsAt": "2026-07-01T10:00:00Z"
            }]
        });
        let alerts = Alert::from_payload(&payload).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].name, "DBConnPoolExhausted");
        assert_eq!(alerts[0].service, "orders-db");
        assert_eq!(alerts[0].summary, "connection pool saturated");
    }

    #[test]
    fn tokenizer_splits_camel_case() {
        let toks = tokenize("CheckoutHighErrorRate on checkout-api");
        assert!(toks.contains(&"checkout".to_string()));
        assert!(toks.contains(&"error".to_string()));
        assert!(toks.contains(&"api".to_string()));
    }
}
