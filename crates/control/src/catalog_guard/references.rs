use super::model;
use crate::AppState;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

pub(super) async fn find(state: &AppState, name: &str, dag: bool) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for (source, value) in state.fleet.catalog_reference_candidates(name).await? {
        if source.starts_with("definition:catalog_retired:") {
            continue;
        }
        if dag && source == format!("definition:dag:{name}") {
            continue;
        }
        if model::references(&value, name, dag) {
            found.push(source);
        }
    }
    for schedule in state.store.list_schedules().await? {
        if model::references(&serde_json::to_value(&schedule.job)?, name, dag) {
            found.push(format!("schedule:{}", schedule.job.id));
        }
    }
    for todo in state.projects.list_todos(None).await? {
        let value = serde_json::json!({"capability_id":todo.capability_id,"executor_ref":todo.executor_ref,
            "executor_spec":todo.executor_spec,"plan_md":todo.plan_md,"draft":todo.draft});
        if model::references(&value, name, dag) {
            found.push(format!("todo:{}", todo.id));
        }
    }
    let config = opencoder_core::Config::load(&state.workdir)?;
    // Never expose configuration values or file contents in conflict responses.
    if model::references(&serde_json::to_value(&config)?, name, dag) {
        found.push("server-config".into());
    }
    if let Some(root) = config.agent.agents_dir {
        let name = name.to_owned();
        found.extend(tokio::task::spawn_blocking(move || files(&root, &name, dag)).await??);
    }
    found.sort();
    found.dedup();
    Ok(found)
}

fn files(root: &Path, name: &str, dag: bool) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .with_context(|| format!("read catalog resources: {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let meta = entry.file_type()?;
            if meta.is_symlink() {
                // Symlinks require explicit review; never silently traverse outside the root.
                found.push(format!(
                    "unverified-agent-link:{}",
                    path.strip_prefix(root)?.display()
                ));
                continue;
            }
            if meta.is_dir() {
                pending.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !matches!(
                extension,
                "" | "json" | "md" | "yaml" | "yml" | "toml" | "py" | "sh" | "mjs" | "js"
            ) {
                continue;
            }
            if entry.metadata()?.len() > 4 * 1024 * 1024 {
                found.push(format!(
                    "unverified-agent-file:{}",
                    path.strip_prefix(root)?.display()
                ));
                continue;
            }
            let bytes = std::fs::read(&path)?;
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if model::references(&Value::String(text.into()), name, dag) {
                    found.push(format!("agent-file:{}", path.strip_prefix(root)?.display()));
                }
            }
        }
    }
    Ok(found)
}
