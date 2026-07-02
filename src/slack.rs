use anyhow::{bail, Context, Result};
use serde_json::json;
use tracing::info;

/// Post a message to a Slack incoming webhook. With no webhook configured we
/// dry-run: the message is logged so the pipeline stays fully testable.
pub async fn post(http: &reqwest::Client, webhook_url: Option<&str>, text: &str) -> Result<()> {
    match webhook_url {
        Some(url) => {
            let resp = http
                .post(url)
                .json(&json!({ "text": text }))
                .send()
                .await
                .context("posting to Slack webhook")?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                bail!("Slack webhook returned {status}: {body}");
            }
            info!("posted brief to Slack");
            Ok(())
        }
        None => {
            info!("Slack webhook not configured — dry run. Message follows:\n{text}");
            Ok(())
        }
    }
}
