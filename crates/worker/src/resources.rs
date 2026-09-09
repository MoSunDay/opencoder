use anyhow::{bail, Context, Result};
use opencoder_core::agent::{AgentMeta, ResourceMeta, AGENT_CATEGORIES};
use std::path::{Path, PathBuf};

pub(crate) fn check_mount(path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let path = path
        .canonicalize()
        .context("configured agent resource mount unavailable")?;
    #[cfg(target_os = "linux")]
    {
        let mounts = std::fs::read_to_string("/proc/self/mountinfo")?;
        let mount = mounts
            .lines()
            .filter_map(|line| {
                let (left, right) = line.split_once(" - ")?;
                let fields: Vec<_> = left.split_whitespace().collect();
                let point = PathBuf::from(fields.get(4)?.replace("\\040", " "));
                if !path.starts_with(&point) {
                    return None;
                }
                Some((
                    point.components().count(),
                    fields.get(5)?.to_string(),
                    right.split_whitespace().next()?.to_string(),
                ))
            })
            .max_by_key(|(length, _, _)| *length);
        if !mount.is_some_and(|(_, options, kind)| {
            kind.starts_with("nfs") && options.split(',').any(|o| o == "ro")
        }) {
            bail!("agent.agents_dir must point to a mounted read-only NFS export");
        }
        std::fs::read_dir(path)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!(
            "read-only NFS mount verification for {} requires Linux /proc/self/mountinfo",
            path.display()
        );
    }
    Ok(())
}

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
    for item in std::fs::read_dir(&source)? {
        let item = item?;
        let name = item.file_name();
        let path = item.path();
        if item.file_type()?.is_symlink() {
            bail!(
                "agent resource entries cannot be symlinks: {}",
                path.display()
            );
        }
        if !path.is_dir() {
            continue;
        }
        let name = name.to_string_lossy();
        if AGENT_CATEGORIES.contains(&name.as_ref()) {
            for resource in std::fs::read_dir(&path)? {
                let resource = resource?;
                if resource.file_type()?.is_symlink() {
                    bail!(
                        "agent resource entries cannot be symlinks: {}",
                        resource.path().display()
                    );
                }
                if !resource.path().is_dir() {
                    continue;
                }
                let meta_path = resource.path().join("meta.json");
                let raw = std::fs::read(&meta_path)?;
                let meta: ResourceMeta = serde_json::from_slice(&raw)?;
                if meta.current == 0 {
                    bail!("resource has no active version: {}", meta_path.display());
                }
                let target = staging.join(name.as_ref()).join(resource.file_name());
                opencoder_core::share_fs::durable_create_dir_all(&target)?;
                std::fs::write(target.join("meta.json"), raw)?;
                std::fs::File::open(target.join("meta.json"))?.sync_all()?;
                std::fs::File::open(&target)?.sync_all()?;
                let version = format!("v{}", meta.current);
                let version_root = resource.path().join(&version);
                if std::fs::symlink_metadata(&version_root)?
                    .file_type()
                    .is_symlink()
                {
                    bail!(
                        "resource version root cannot be a symlink: {}",
                        version_root.display()
                    );
                }
                let original = version_root.canonicalize()?;
                if !original.starts_with(&source) {
                    bail!("resource version escaped configured resource root");
                }
                copy_version(&original, &target.join(version))?;
            }
        } else {
            let raw = std::fs::read(path.join("meta.json"))?;
            let _: AgentMeta = serde_json::from_slice(&raw)?;
            let target = staging.join(name.as_ref());
            opencoder_core::share_fs::durable_create_dir_all(&target)?;
            std::fs::write(target.join("meta.json"), raw)?;
            std::fs::File::open(target.join("meta.json"))?.sync_all()?;
            std::fs::File::open(&target)?.sync_all()?;
        }
    }
    for entry in std::fs::read_dir(&staging)? {
        let path = entry?.path();
        if AGENT_CATEGORIES.contains(&path.file_name().unwrap().to_string_lossy().as_ref()) {
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
    Ok(Some(destination.into()))
}

fn copy_version(source: &Path, destination: &Path) -> Result<()> {
    opencoder_core::share_fs::durable_create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            bail!(
                "resource versions cannot contain symlinks: {}",
                path.display()
            );
        }
        if path.is_dir() {
            copy_version(&path, &destination.join(entry.file_name()))?;
        } else {
            let target = destination.join(entry.file_name());
            std::fs::copy(&path, &target)?;
            std::fs::File::open(target)?.sync_all()?;
        }
    }
    std::fs::File::open(destination)?.sync_all()?;
    Ok(())
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
