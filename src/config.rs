use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub slack: SlackConfig,
    #[serde(default)]
    pub repo: RepoConfig,
    #[serde(default)]
    pub runbooks: RunbooksConfig,
    #[serde(default)]
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    /// "auto" (real when ANTHROPIC_API_KEY is set, else mock), "real", or "mock"
    #[serde(default = "default_llm_mode")]
    pub mode: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Filled from the ANTHROPIC_API_KEY env var; never put the key in the file.
    #[serde(skip)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SlackConfig {
    /// Incoming webhook URL. Falls back to SLACK_WEBHOOK_URL env var.
    /// When unset, briefs are printed to the log (dry-run).
    #[serde(default)]
    pub webhook_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    /// Path to the git repository to analyze for suspect commits.
    #[serde(default = "default_repo_path")]
    pub path: String,
    /// How far back to look for candidate commits.
    #[serde(default = "default_lookback_hours")]
    pub lookback_hours: u64,
    #[serde(default = "default_max_commits")]
    pub max_commits: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunbooksConfig {
    #[serde(default = "default_runbooks_dir")]
    pub dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    #[serde(default = "default_incidents_dir")]
    pub incidents_dir: String,
    #[serde(default = "default_postmortems_dir")]
    pub postmortems_dir: String,
}

fn default_bind() -> String {
    "127.0.0.1:8080".into()
}
fn default_llm_mode() -> String {
    "auto".into()
}
fn default_model() -> String {
    "claude-opus-4-8".into()
}
fn default_max_tokens() -> u32 {
    8000
}
fn default_repo_path() -> String {
    ".".into()
}
fn default_lookback_hours() -> u64 {
    48
}
fn default_max_commits() -> usize {
    30
}
fn default_runbooks_dir() -> String {
    "runbooks".into()
}
fn default_incidents_dir() -> String {
    "data/incidents".into()
}
fn default_postmortems_dir() -> String {
    "postmortems".into()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
        }
    }
}
impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            mode: default_llm_mode(),
            model: default_model(),
            max_tokens: default_max_tokens(),
            api_key: None,
        }
    }
}
impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            path: default_repo_path(),
            lookback_hours: default_lookback_hours(),
            max_commits: default_max_commits(),
        }
    }
}
impl Default for RunbooksConfig {
    fn default() -> Self {
        Self {
            dir: default_runbooks_dir(),
        }
    }
}
impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            incidents_dir: default_incidents_dir(),
            postmortems_dir: default_postmortems_dir(),
        }
    }
}

impl Config {
    /// Load from a TOML file if it exists (defaults otherwise), then apply env overrides.
    pub fn load(path: &str) -> Result<Self> {
        let mut cfg: Config = if Path::new(path).exists() {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading config file {path}"))?;
            toml::from_str(&raw).with_context(|| format!("parsing config file {path}"))?
        } else {
            Config::default()
        };

        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                cfg.llm.api_key = Some(key);
            }
        }
        if cfg.slack.webhook_url.is_none() {
            if let Ok(url) = std::env::var("SLACK_WEBHOOK_URL") {
                if !url.is_empty() {
                    cfg.slack.webhook_url = Some(url);
                }
            }
        }
        Ok(cfg)
    }

    /// Resolve the effective LLM mode: true = call the real Claude API.
    pub fn llm_use_real(&self) -> bool {
        match self.llm.mode.as_str() {
            "real" => true,
            "mock" => false,
            _ => self.llm.api_key.is_some(),
        }
    }
}
