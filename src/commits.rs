use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use tokio::process::Command;

const RECORD_SEP: char = '\u{1e}';
const FIELD_SEP: char = '\u{1f}';

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub files: Vec<String>,
}

/// Recent commits in the configured repo, newest first, via the git CLI.
pub async fn recent_commits(repo: &Path, lookback_hours: u64, limit: usize) -> Result<Vec<Commit>> {
    let since = format!("{lookback_hours} hours ago");
    let pretty = format!("--pretty=format:{RECORD_SEP}%H{FIELD_SEP}%an{FIELD_SEP}%aI{FIELD_SEP}%s");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "--name-only", "--since", &since, "-n"])
        .arg(limit.to_string())
        .arg(&pretty)
        .output()
        .await
        .context("running git (is git installed?)")?;

    if !output.status.success() {
        bail!(
            "git log failed for {}: {}",
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_log(&stdout))
}

fn parse_log(raw: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    for record in raw.split(RECORD_SEP) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let mut lines = record.lines();
        let Some(header) = lines.next() else { continue };
        let fields: Vec<&str> = header.split(FIELD_SEP).collect();
        if fields.len() < 4 {
            continue;
        }
        let files = lines
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        commits.push(Commit {
            hash: fields[0].to_string(),
            author: fields[1].to_string(),
            date: fields[2].to_string(),
            message: fields[3].to_string(),
            files,
        });
    }
    commits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_log_records() {
        let raw = format!(
            "{RECORD_SEP}abc{FIELD_SEP}Ada{FIELD_SEP}2026-07-01T10:00:00+00:00{FIELD_SEP}Fix bug\nsrc/a.rs\nsrc/b.rs\n\
             {RECORD_SEP}def{FIELD_SEP}Bob{FIELD_SEP}2026-07-01T09:00:00+00:00{FIELD_SEP}Add feature\nsrc/c.rs\n"
        );
        let commits = parse_log(&raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc");
        assert_eq!(commits[0].files, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(commits[1].message, "Add feature");
    }
}
