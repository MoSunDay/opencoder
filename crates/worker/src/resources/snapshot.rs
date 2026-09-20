use anyhow::{bail, Context, Result};
use opencoder_core::agent::{AgentMeta, AGENT_CATEGORIES};
use std::path::{Path, PathBuf};

#[path = "copy.rs"]
mod copy;

/// Freeze cards and current resource versions in the node snapshot so publish,
/// rollback and removal cannot change an execution that was already accepted.
pub(crate) fn pin(source: Option<&Path>, destination: &Path) -> Result<Option<PathBuf>> {
    if destination.exists() {
        if std::fs::symlink_metadata(destination)?
            .file_type()
            .is_symlink()
            || !destination.is_dir()
        {
            bail!("resource snapshot destination must be a real directory");
        }
        return Ok(Some(destination.into()));
    }
    let Some(source) = source else {
        opencoder_core::share_fs::durable_create_dir_all(destination)?;
        std::fs::File::open(destination)?.sync_all()?;
        std::fs::File::open(destination.parent().unwrap())?.sync_all()?;
        return Ok(Some(destination.into()));
    };
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("resource snapshot source unavailable: {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("agent resource mount root cannot be a symlink");
    }
    if !metadata.is_dir() {
        bail!(
            "resource snapshot source must be a directory: {}",
            source.display()
        );
    }
    let source = source.canonicalize()?;
    let staging = destination.with_extension(format!("staging-{}", ulid::Ulid::new()));
    opencoder_core::share_fs::durable_create_dir_all(&staging)?;
    let _cleanup = Staging(staging.clone());
    let started = std::time::Instant::now();
    let entries = copy::all(&source, &staging)?;
    for entry in std::fs::read_dir(&staging)? {
        let path = entry?.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if AGENT_CATEGORIES.contains(&name.as_ref()) {
            continue;
        }
        let meta: AgentMeta = serde_json::from_slice(&std::fs::read(path.join("meta.json"))?)?;
        for (cat, name) in [
            ("prompts", meta.current.prompt),
            ("skills", meta.current.skills),
            ("tools", meta.current.tools),
            ("memory", meta.current.memory),
        ] {
            if let Some(name) = name {
                if !staging.join(cat).join(&name).join("meta.json").is_file() {
                    bail!("agent resource reference missing: {cat}/{name}");
                }
            }
        }
    }
    std::fs::File::open(&staging)?.sync_all()?;
    std::fs::rename(staging, destination)?;
    std::fs::File::open(destination.parent().unwrap())?.sync_all()?;
    tracing::info!(entries, elapsed_ms = started.elapsed().as_millis(), path = %destination.display(), "agent resource snapshot frozen");
    Ok(Some(destination.into()))
}

// Failed copies never accumulate partially published resource directories.
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.exists() {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                tracing::error!(path = %self.0.display(), %error, "remove resource staging directory");
            }
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
