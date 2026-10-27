mod resources;

use super::model;
use crate::AppState;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{io::Read, path::Path};

pub(super) async fn find(state: &AppState, name: &str, dag: bool) -> Result<Vec<String>> {
    scan(state, name, dag, false).await
}

pub(super) async fn find_agent(state: &AppState, name: &str) -> Result<Vec<String>> {
    scan(state, name, false, true).await
}

fn matches(value: &Value, name: &str, dag: bool, agent: bool) -> bool {
    if agent {
        super::agents::references(value, name)
    } else {
        model::references(value, name, dag)
    }
}

async fn scan(state: &AppState, name: &str, dag: bool, agent: bool) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for (source, value) in state.fleet.catalog_reference_candidates(name).await? {
        if source.starts_with("definition:catalog_retired:") {
            continue;
        }
        if dag && source == format!("definition:dag:{name}") {
            continue;
        }
        if matches(&value, name, dag, agent) {
            found.push(source);
        }
    }
    for schedule in state.store.list_schedules().await? {
        if matches(&serde_json::to_value(&schedule.job)?, name, dag, agent) {
            found.push(format!("schedule:{}", schedule.job.id));
        }
    }
    for todo in state.projects.list_todos(None).await? {
        let value = serde_json::json!({"capability_id":todo.capability_id,"executor_ref":todo.executor_ref,
            "executor_spec":todo.executor_spec,"plan_md":todo.plan_md,"draft":todo.draft});
        if matches(&value, name, dag, agent) {
            found.push(format!("todo:{}", todo.id));
        }
    }
    let config = opencoder_core::Config::load(&state.workdir)?;
    // Never expose configuration values or file contents in conflict responses.
    if matches(&serde_json::to_value(&config)?, name, dag, agent) {
        found.push("server-config".into());
    }
    if agent {
        for capability in state.store.list_brain_capabilities().await? {
            if matches(&serde_json::to_value(&capability)?, name, dag, agent) {
                found.push(format!("capability:{}", capability.capability.id));
            }
        }
    }
    if let Some(root) = config
        .agent
        .agents_dir
        .or_else(opencoder_core::agent::agents_dir)
    {
        let name = name.to_owned();
        // The blocking walk drives the async resource-database engine through
        // the caller's runtime handle; source files stay untouched.
        let handle = tokio::runtime::Handle::current();
        found.extend(
            tokio::task::spawn_blocking(move || files(&handle, &root, &name, dag, agent)).await??,
        );
    }
    found.sort();
    found.dedup();
    Ok(found)
}

fn files(
    handle: &tokio::runtime::Handle,
    root: &Path,
    name: &str,
    dag: bool,
    agent: bool,
) -> Result<Vec<String>> {
    anyhow::ensure!(
        std::fs::symlink_metadata(root)?.file_type().is_dir(),
        "agent resource root is not a directory"
    );
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .with_context(|| format!("read catalog resources: {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let meta = entry.file_type()?;
            if agent && path == root.join(name).join("meta.json") {
                continue;
            }
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
                found.push(format!(
                    "unverified-agent-file:{}",
                    path.strip_prefix(root)?.display()
                ));
                continue;
            }
            if resources::sidecar(&path)? {
                continue;
            }
            // Tools and skill packages accept arbitrary file names. An unknown
            // extension or encoding cannot prove that a resource is unused.
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            let text = std::str::from_utf8(&bytes)
                .ok()
                .filter(|text| !text.contains('\0'));
            if bytes.len() <= 4 * 1024 * 1024 && text.is_none() {
                if let Some(values) = handle.block_on(resources::values(&path, &bytes))? {
                    if values.iter().any(|value| matches(value, name, dag, agent)) {
                        found.push(format!("agent-file:{}", path.strip_prefix(root)?.display()));
                    }
                    continue;
                }
            }
            if bytes.len() > 4 * 1024 * 1024 || text.is_none() {
                found.push(format!(
                    "unverified-agent-file:{}",
                    path.strip_prefix(root)?.display()
                ));
                continue;
            }
            if let Some(text) = text {
                let mut value = Value::String(text.into());
                let relative = path.strip_prefix(root)?;
                // Card fields name shared pools and execution configuration;
                // they do not declare dependencies on other Agent cards.
                // Unknown fields remain checked for forward compatibility.
                if agent
                    && relative.components().count() == 2
                    && path.file_name().is_some_and(|file| file == "meta.json")
                    && relative.components().next().is_some_and(|part| {
                        part.as_os_str().to_str().is_some_and(|card| {
                            opencoder_core::agent::validate_agent_name(card).is_ok()
                        })
                    })
                {
                    value = serde_json::from_str(text)?;
                    let fields = value.as_object_mut().context("invalid Agent metadata")?;
                    for field in [
                        "name",
                        "current",
                        "history",
                        "references",
                        "harness",
                        "harness_profile",
                        "created_at",
                        "updated_at",
                    ] {
                        fields.remove(field);
                    }
                }
                // A pool's own name identifies that pool, not an Agent binding.
                // Keep all remaining metadata fields in the reference scan.
                if agent
                    && path.file_name().is_some_and(|file| file == "meta.json")
                    && path
                        .strip_prefix(root)?
                        .components()
                        .next()
                        .is_some_and(|part| {
                            ["prompts", "skills", "tools", "memory"]
                                .iter()
                                .any(|category| part.as_os_str() == *category)
                        })
                {
                    value = serde_json::from_str(text)?;
                    value
                        .as_object_mut()
                        .context("invalid resource metadata")?
                        .remove("name");
                }
                if matches(&value, name, dag, agent) {
                    found.push(format!("agent-file:{}", path.strip_prefix(root)?.display()));
                }
            }
        }
    }
    Ok(found)
}
