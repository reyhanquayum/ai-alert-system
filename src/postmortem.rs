use crate::incident::Incident;
use crate::llm::{Llm, Task};
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Generate the postmortem markdown for a resolved incident and write it to disk.
pub async fn generate_and_save(llm: &Llm, incident: &Incident, dir: &Path) -> Result<PathBuf> {
    let context = json!({ "incident": incident });
    let markdown = llm.generate(Task::Postmortem, &context).await?;

    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{}.md", incident.id));
    std::fs::write(&path, &markdown).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}
