use crate::alert::Alert;
use serde::Serialize;

/// Point-in-time metrics for the affected service. In production this would
/// come from Prometheus/Datadog — `snapshot_for` is the seam to swap in a real
/// provider. The mock derives plausible numbers from the alert severity so the
/// downstream impact estimate has something concrete to reason over.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub source: String,
    pub error_rate_pct: f64,
    pub requests_per_min: f64,
    pub p95_latency_ms: f64,
}

pub fn snapshot_for(alert: &Alert) -> MetricsSnapshot {
    let (error_rate_pct, requests_per_min, p95_latency_ms) = match alert.severity.as_str() {
        "critical" => (24.0, 4200.0, 2600.0),
        "warning" => (4.5, 3800.0, 900.0),
        _ => (0.8, 3500.0, 320.0),
    };
    MetricsSnapshot {
        source: "mock-metrics (wire up Prometheus here)".into(),
        error_rate_pct,
        requests_per_min,
        p95_latency_ms,
    }
}
