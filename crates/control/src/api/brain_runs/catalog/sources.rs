//! Filesystem discovery for synchronization must distinguish missing from unreadable.
use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn directories(path: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => {
            return Err(error).with_context(|| format!("read catalog directory {}", path.display()))
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            names.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("catalog directory name is not UTF-8"))?,
            );
        }
    }
    names.sort();
    Ok(names)
}

pub(super) fn agents() -> Result<Vec<opencoder_core::Agent>> {
    let Some(root) = opencoder_core::agent::agents_dir() else {
        return Ok(vec![]);
    };
    // Validate directory enumeration before using the established reserved-name filter.
    directories(&root)?;
    opencoder_core::agent::list_agents()
        .into_iter()
        .map(|name| {
            opencoder_core::resolve_agent(&name)
                .with_context(|| format!("agent definition unavailable: {name}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_source_is_not_treated_as_an_empty_catalog() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        assert!(directories(&source).unwrap().is_empty());
        std::fs::write(&source, "invalid directory").unwrap();
        assert!(directories(&source).is_err());
    }

    #[tokio::test]
    async fn reading_a_missing_registered_workflow_does_not_create_source_directories() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let share = root.path().join("share");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(
            work.join("opencoder.json"),
            serde_json::json!({"agent":{
                "agents_dir":root.path().join("agents"),"share_dir":share
            }})
            .to_string(),
        )
        .unwrap();
        let state = crate::new_state(work, root.path().join("data"), None)
            .await
            .unwrap();
        let target = serde_json::json!({"kind":"todos","target":"missing/v1"});
        assert!(super::super::registered_definition(&state, &target, false)
            .await
            .is_err());
        assert!(!share.exists());
    }
}
