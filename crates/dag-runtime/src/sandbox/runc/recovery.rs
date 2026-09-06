//! Startup cleanup for containers proven to belong to this node's bundles.

use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Delete residual containers below exact node-owned bundle roots before a
/// restarted worker can register as ready. Sources are retained; only runc's
/// private runtime state is changed through `runc delete --force`.
pub async fn cleanup_owned_containers(bundle_roots: &[PathBuf]) -> Result<usize> {
    let mut cleaned = 0;
    let mut seen = BTreeSet::new();
    for bundles in bundle_roots {
        if !seen.insert(bundles.clone()) || !bundles.exists() {
            continue;
        }
        require_real_dir(bundles, "bundle root")?;
        for run in real_child_dirs(bundles, "bundle root")? {
            for step in real_child_dirs(&run, "bundle run")? {
                let state = step.join("runc-state");
                if !state.exists() {
                    continue;
                }
                require_real_dir(&state, "runc state root")?;
                for entry in std::fs::read_dir(&state)? {
                    let entry = entry?;
                    if entry.file_type()?.is_symlink() || !entry.file_type()?.is_dir() {
                        bail!("unknown runc state entry: {}", entry.path().display());
                    }
                    let id = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("non-UTF8 runc container id"))?;
                    validate_id(&id)?;
                    super::delete_force(&state, &id)
                        .await
                        .with_context(|| format!("cleanup owned container {id}"))?;
                    cleaned += 1;
                }
            }
        }
    }
    Ok(cleaned)
}

fn real_child_dirs(root: &Path, label: &str) -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_dir() {
            bail!("unknown {label} entry: {}", entry.path().display());
        }
        directories.push(entry.path());
    }
    Ok(directories)
}

fn require_real_dir(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} must be a real directory: {}",
        path.display()
    );
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 255
            && id
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || value == '-' || value == '_'),
        "invalid owned runc container id"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_owned_roots_need_no_runc() {
        let directory = tempfile::tempdir().unwrap();
        let bundles = directory.path().join("bundles");
        std::fs::create_dir_all(&bundles).unwrap();
        assert_eq!(cleanup_owned_containers(&[bundles]).await.unwrap(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_state_fails_closed_without_touching_target() {
        let directory = tempfile::tempdir().unwrap();
        let bundles = directory.path().join("bundles");
        let step = bundles.join("run/step");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&step).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), "safe").unwrap();
        std::os::unix::fs::symlink(&outside, step.join("runc-state")).unwrap();
        let error = cleanup_owned_containers(&[bundles])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("must be a real directory"), "{error}");
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "safe"
        );
    }
}
