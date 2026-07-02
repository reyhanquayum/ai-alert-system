use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    SuspectCommit,
    SelectRunbook,
    EstimateImpact,
    ComposeBrief,
    Postmortem,
}

#[derive(Debug, Clone)]
enum Backend {
    Real { api_key: String },
    Mock,
}

/// Claude Messages API client (raw HTTP — there is no official Rust SDK).
/// Falls back to a deterministic mock backend so the whole pipeline can run
/// offline / in tests / without an API key.
#[derive(Debug, Clone)]
pub struct Llm {
    backend: Backend,
    http: reqwest::Client,
    model: String,
    max_tokens: u32,
}

impl Llm {
    pub fn real(api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            backend: Backend::Real { api_key },
            http: reqwest::Client::new(),
            model,
            max_tokens,
        }
    }

    pub fn mock() -> Self {
        Self {
            backend: Backend::Mock,
            http: reqwest::Client::new(),
            model: "mock".into(),
            max_tokens: 0,
        }
    }

    /// Structured task: the model must answer with a single JSON object.
    pub async fn analyze(&self, task: Task, context: &Value) -> Result<Value> {
        match &self.backend {
            Backend::Mock => mock::analyze(task, context),
            Backend::Real { .. } => {
                let text = self.complete(task, context).await?;
                extract_json(&text)
                    .with_context(|| format!("model reply was not parseable JSON: {text}"))
            }
        }
    }

    /// Freeform task: the model answers with markdown text.
    pub async fn generate(&self, task: Task, context: &Value) -> Result<String> {
        match &self.backend {
            Backend::Mock => mock::generate(task, context),
            Backend::Real { .. } => self.complete(task, context).await,
        }
    }

    async fn complete(&self, task: Task, context: &Value) -> Result<String> {
        let Backend::Real { api_key } = &self.backend else {
            unreachable!("complete() is only called on the real backend");
        };
        let body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "thinking": {"type": "adaptive"},
            "system": system_prompt(task),
            "messages": [{"role": "user", "content": user_prompt(task, context)}],
        });

        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
            .send()
            .await
            .context("sending request to the Claude API")?;

        let status = resp.status();
        let raw: Value = resp.json().await.context("reading Claude API response body")?;
        if !status.is_success() {
            let msg = raw
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            bail!("Claude API error {status}: {msg}");
        }

        let parsed: ApiResponse = serde_json::from_value(raw)?;
        if parsed.stop_reason.as_deref() == Some("refusal") {
            bail!("Claude declined the request (stop_reason: refusal)");
        }
        let text: String = parsed
            .content
            .iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            bail!("Claude returned an empty response (stop_reason: {:?})", parsed.stop_reason);
        }
        Ok(text)
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

/// Pull the first top-level JSON object out of a model reply (tolerates code fences / prose).
fn extract_json(text: &str) -> Result<Value> {
    let start = text.find('{').ok_or_else(|| anyhow!("no JSON object in reply"))?;
    let end = text.rfind('}').ok_or_else(|| anyhow!("no JSON object in reply"))?;
    Ok(serde_json::from_str(&text[start..=end])?)
}

fn system_prompt(task: Task) -> &'static str {
    match task {
        Task::SuspectCommit => {
            "You are an SRE assistant doing incident triage. Given a production alert and a \
             list of recent commits (message, author, changed files), identify the commit most \
             likely to have caused the incident. Weigh file paths and commit messages against \
             the failing service and symptom. Reply with ONLY a JSON object: \
             {\"hash\": \"<full hash of the chosen commit, or null if none is plausible>\", \
             \"confidence\": \"high|medium|low\", \"reasoning\": \"<2-3 sentences>\"}"
        }
        Task::SelectRunbook => {
            "You are an SRE assistant. Given a production alert and a list of candidate \
             runbooks (with excerpts), pick the single most relevant runbook and extract the \
             first concrete remediation steps a responder should take. Reply with ONLY a JSON \
             object: {\"path\": \"<path of chosen runbook>\", \"reasoning\": \"<1-2 sentences>\", \
             \"key_steps\": [\"<step>\", ...]} (3-5 steps, imperative voice)."
        }
        Task::EstimateImpact => {
            "You are an SRE assistant estimating user impact during an incident. Given an \
             alert and a metrics snapshot, estimate the blast radius. Be concrete and honest \
             about uncertainty. Reply with ONLY a JSON object: \
             {\"severity_assessment\": \"<one line>\", \"affected_users\": \"<estimate, e.g. '~12k users/hr'>\", \
             \"narrative\": \"<2-4 sentences on who is affected and how>\"}"
        }
        Task::ComposeBrief => {
            "You are an SRE assistant writing an incident brief for a Slack channel. Given the \
             incident record (alert, suspect commit, runbook, impact estimate), write a crisp \
             brief in Slack mrkdwn (use *bold*, not **bold**; use `code` for hashes/paths). \
             Structure: headline with incident id + severity, then What's happening, Likely \
             cause, Impact, Next steps (from the runbook). Keep it under 250 words. Reply with \
             ONLY the brief text — no preamble."
        }
        Task::Postmortem => {
            "You are an SRE assistant writing a blameless postmortem. Given the full incident \
             record (alert, timeline, suspect commit, runbook used, impact, resolution note), \
             write a markdown postmortem with these sections: # Postmortem: <title>, \
             ## Summary, ## Timeline, ## Root Cause, ## Impact, ## What Went Well, \
             ## What Went Poorly, ## Action Items (checklist). Be factual; only use \
             information from the record; mark inferences as such. Reply with ONLY the \
             markdown document."
        }
    }
}

fn user_prompt(task: Task, context: &Value) -> String {
    let label = match task {
        Task::SuspectCommit => "Alert and recent commits",
        Task::SelectRunbook => "Alert and candidate runbooks",
        Task::EstimateImpact => "Alert and metrics snapshot",
        Task::ComposeBrief => "Incident record",
        Task::Postmortem => "Full incident record",
    };
    format!(
        "{label}:\n```json\n{}\n```",
        serde_json::to_string_pretty(context).unwrap_or_else(|_| context.to_string())
    )
}

/// Deterministic offline backend. Produces the same JSON shapes as the real
/// model using simple heuristics, so demos and tests run without a key.
mod mock {
    use super::*;
    use crate::alert::tokenize;

    fn overlap(a: &[String], b: &[String]) -> usize {
        a.iter().filter(|t| b.contains(t)).count()
    }

    fn context_alert_text(context: &Value) -> String {
        let alert = &context["alert"];
        format!(
            "{} {} {} {}",
            alert["name"].as_str().unwrap_or(""),
            alert["service"].as_str().unwrap_or(""),
            alert["summary"].as_str().unwrap_or(""),
            alert["description"].as_str().unwrap_or("")
        )
    }

    pub fn analyze(task: Task, context: &Value) -> Result<Value> {
        match task {
            Task::SuspectCommit => suspect_commit(context),
            Task::SelectRunbook => select_runbook(context),
            Task::EstimateImpact => estimate_impact(context),
            _ => bail!("task {:?} is not a JSON analysis task", task),
        }
    }

    fn suspect_commit(context: &Value) -> Result<Value> {
        let alert_tokens = tokenize(&context_alert_text(context));
        let empty = Vec::new();
        let commits = context["commits"].as_array().unwrap_or(&empty);
        if commits.is_empty() {
            return Ok(json!({
                "hash": null,
                "confidence": "low",
                "reasoning": "No recent commits were found in the lookback window."
            }));
        }
        let best = commits
            .iter()
            .max_by_key(|c| {
                let text = format!(
                    "{} {}",
                    c["message"].as_str().unwrap_or(""),
                    c["files"]
                        .as_array()
                        .map(|f| f.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default()
                );
                overlap(&tokenize(&text), &alert_tokens)
            })
            .unwrap();
        let score = {
            let text = format!(
                "{} {}",
                best["message"].as_str().unwrap_or(""),
                best["files"]
                    .as_array()
                    .map(|f| f.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default()
            );
            overlap(&tokenize(&text), &alert_tokens)
        };
        let confidence = if score >= 3 { "high" } else if score >= 1 { "medium" } else { "low" };
        Ok(json!({
            "hash": best["hash"],
            "confidence": confidence,
            "reasoning": format!(
                "[mock heuristic] Commit message and changed files share {} keyword(s) with the \
                 alert ({}), making it the strongest candidate in the lookback window.",
                score,
                best["message"].as_str().unwrap_or("?")
            )
        }))
    }

    fn select_runbook(context: &Value) -> Result<Value> {
        let empty = Vec::new();
        let candidates = context["candidates"].as_array().unwrap_or(&empty);
        if candidates.is_empty() {
            bail!("no runbook candidates provided");
        }
        let best = candidates
            .iter()
            .max_by(|a, b| {
                let sa = a["score"].as_f64().unwrap_or(0.0);
                let sb = b["score"].as_f64().unwrap_or(0.0);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let excerpt = best["excerpt"].as_str().unwrap_or("");
        let clean = |l: &str| {
            l.trim_start_matches(['-', '*', ' '])
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')')
                .trim()
                .to_string()
        };
        // Numbered lines are remediation steps; bulleted lines are usually symptoms.
        let mut steps: Vec<String> = excerpt
            .lines()
            .map(str::trim)
            .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .take(4)
            .map(clean)
            .filter(|l| !l.is_empty())
            .collect();
        if steps.is_empty() {
            steps = excerpt
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with("- ") || l.starts_with("* "))
                .take(4)
                .map(clean)
                .filter(|l| !l.is_empty())
                .collect();
        }
        Ok(json!({
            "path": best["path"],
            "reasoning": "[mock heuristic] Highest keyword overlap between the alert text and this runbook.",
            "key_steps": if steps.is_empty() { vec!["Open the runbook and follow the diagnosis section.".to_string()] } else { steps }
        }))
    }

    fn estimate_impact(context: &Value) -> Result<Value> {
        let severity = context["alert"]["severity"].as_str().unwrap_or("warning");
        let service = context["alert"]["service"].as_str().unwrap_or("unknown");
        let error_rate = context["metrics"]["error_rate_pct"].as_f64().unwrap_or(0.0);
        let rpm = context["metrics"]["requests_per_min"].as_f64().unwrap_or(0.0);
        let failing_per_min = rpm * error_rate / 100.0;
        let (assessment, users) = match severity {
            "critical" => (
                format!("High — {error_rate:.0}% of requests to {service} are failing"),
                format!("~{:.0}k affected requests/hour", failing_per_min * 60.0 / 1000.0),
            ),
            "warning" => (
                format!("Moderate — degraded behavior on {service}"),
                format!("~{:.1}k affected requests/hour", failing_per_min * 60.0 / 1000.0),
            ),
            _ => ("Low — no clear user-facing impact yet".to_string(), "minimal".to_string()),
        };
        Ok(json!({
            "severity_assessment": assessment,
            "affected_users": users,
            "narrative": format!(
                "[mock heuristic] At ~{rpm:.0} req/min with a {error_rate:.1}% error rate, roughly \
                 {failing_per_min:.0} requests/min against {service} are failing. Users hitting the \
                 affected paths see errors or timeouts until mitigation lands."
            )
        }))
    }

    pub fn generate(task: Task, context: &Value) -> Result<String> {
        match task {
            Task::ComposeBrief => Ok(compose_brief(context)),
            Task::Postmortem => Ok(postmortem(context)),
            _ => bail!("task {:?} is not a text generation task", task),
        }
    }

    fn compose_brief(context: &Value) -> String {
        let id = context["incident"]["id"].as_str().unwrap_or("INC-?");
        let alert = &context["incident"]["alert"];
        let name = alert["name"].as_str().unwrap_or("?");
        let severity = alert["severity"].as_str().unwrap_or("?");
        let service = alert["service"].as_str().unwrap_or("?");
        let summary = alert["summary"].as_str().unwrap_or("");

        let mut out = format!(
            ":rotating_light: *{id}: {name}* (severity: {severity})\n*Service:* `{service}`\n*What's happening:* {summary}\n"
        );
        if let Some(sc) = context["incident"]["suspect_commit"].as_object() {
            out.push_str(&format!(
                "*Likely cause:* `{}` — {} ({}) — confidence: {}\n> {}\n",
                sc.get("hash").and_then(|v| v.as_str()).map(|h| &h[..h.len().min(10)]).unwrap_or("?"),
                sc.get("message").and_then(|v| v.as_str()).unwrap_or("?"),
                sc.get("author").and_then(|v| v.as_str()).unwrap_or("?"),
                sc.get("confidence").and_then(|v| v.as_str()).unwrap_or("?"),
                sc.get("reasoning").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
        if let Some(imp) = context["incident"]["impact"].as_object() {
            out.push_str(&format!(
                "*Impact:* {} ({})\n",
                imp.get("severity_assessment").and_then(|v| v.as_str()).unwrap_or("?"),
                imp.get("affected_users").and_then(|v| v.as_str()).unwrap_or("?")
            ));
        }
        if let Some(rb) = context["incident"]["runbook"].as_object() {
            out.push_str(&format!(
                "*Runbook:* {} (`{}`)\n*Next steps:*\n",
                rb.get("title").and_then(|v| v.as_str()).unwrap_or("?"),
                rb.get("path").and_then(|v| v.as_str()).unwrap_or("?")
            ));
            if let Some(steps) = rb.get("key_steps").and_then(|v| v.as_array()) {
                for (i, step) in steps.iter().enumerate() {
                    out.push_str(&format!("{}. {}\n", i + 1, step.as_str().unwrap_or("")));
                }
            }
        }
        out.push_str("_Status: investigating — replies in thread please._");
        out
    }

    fn postmortem(context: &Value) -> String {
        let inc = &context["incident"];
        let id = inc["id"].as_str().unwrap_or("INC-?");
        let name = inc["alert"]["name"].as_str().unwrap_or("?");
        let service = inc["alert"]["service"].as_str().unwrap_or("?");
        let resolution = inc["resolution"].as_str().unwrap_or("(no resolution note)");
        let mut timeline = String::new();
        if let Some(entries) = inc["timeline"].as_array() {
            for e in entries {
                timeline.push_str(&format!(
                    "- {} — {}\n",
                    e["at"].as_str().unwrap_or("?"),
                    e["event"].as_str().unwrap_or("?")
                ));
            }
        }
        let root_cause = inc["suspect_commit"]
            .as_object()
            .map(|sc| {
                format!(
                    "Suspected commit `{}` (\"{}\", by {}): {}",
                    sc.get("hash").and_then(|v| v.as_str()).unwrap_or("?"),
                    sc.get("message").and_then(|v| v.as_str()).unwrap_or("?"),
                    sc.get("author").and_then(|v| v.as_str()).unwrap_or("?"),
                    sc.get("reasoning").and_then(|v| v.as_str()).unwrap_or("")
                )
            })
            .unwrap_or_else(|| "No suspect commit was identified.".to_string());
        let impact = inc["impact"]
            .as_object()
            .map(|i| {
                format!(
                    "{} — {}. {}",
                    i.get("severity_assessment").and_then(|v| v.as_str()).unwrap_or("?"),
                    i.get("affected_users").and_then(|v| v.as_str()).unwrap_or("?"),
                    i.get("narrative").and_then(|v| v.as_str()).unwrap_or("")
                )
            })
            .unwrap_or_else(|| "Impact was not quantified.".to_string());

        format!(
            "# Postmortem: {id} — {name}\n\n\
             _Generated automatically by ai-alert-system (mock backend)._\n\n\
             ## Summary\n\nAn alert (`{name}`) fired for service `{service}`. The incident was \
             investigated automatically and resolved. Resolution: {resolution}\n\n\
             ## Timeline\n\n{timeline}\n\
             ## Root Cause\n\n{root_cause}\n\n\
             ## Impact\n\n{impact}\n\n\
             ## What Went Well\n\n- Automated triage produced a brief within seconds of the alert.\n\n\
             ## What Went Poorly\n\n- (fill in during review)\n\n\
             ## Action Items\n\n- [ ] Confirm the suspect commit and add a regression test\n\
             - [ ] Review alert thresholds for `{service}`\n\
             - [ ] Update the runbook with anything learned in this incident\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_suspect_commit_picks_overlapping_commit() {
        let llm = Llm::mock();
        let ctx = json!({
            "alert": {"name": "CheckoutHighErrorRate", "service": "checkout-api",
                      "summary": "5xx errors on checkout", "severity": "critical"},
            "commits": [
                {"hash": "aaa111", "message": "Update README", "files": ["README.md"]},
                {"hash": "bbb222", "message": "Refactor checkout retry logic", "files": ["src/checkout/retry.rs"]},
                {"hash": "ccc333", "message": "Bump logging level", "files": ["src/log.rs"]}
            ]
        });
        let result = llm.analyze(Task::SuspectCommit, &ctx).await.unwrap();
        assert_eq!(result["hash"], "bbb222");
    }

    #[test]
    fn extract_json_tolerates_fences() {
        let text = "Here you go:\n```json\n{\"a\": 1}\n```";
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
    }
}
