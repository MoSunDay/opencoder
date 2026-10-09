//! Bounded parallel copying; no snapshot becomes visible before every join/fsync.
use anyhow::{bail, Context, Result};
use opencoder_core::agent::{AgentMeta, ResourceMeta, AGENT_CATEGORIES};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Entry {
    source: PathBuf,
    target: PathBuf,
    resource: bool,
}

pub(super) fn all(source: &Path, staging: &Path) -> Result<usize> {
    let mut entries = Vec::new();
    for item in std::fs::read_dir(source)? {
        let item = item?;
        if !entry_type(&item)?.is_dir() {
            continue;
        }
        let name = item.file_name();
        if AGENT_CATEGORIES.contains(&name.to_string_lossy().as_ref()) {
            // Create shared category parents before workers own disjoint children.
            let target = staging.join(&name);
            std::fs::create_dir_all(&target)?;
            for resource in std::fs::read_dir(item.path())? {
                let resource = resource?;
                if resource
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".staging~")
                {
                    continue;
                }
                if entry_type(&resource)?.is_dir() {
                    entries.push(Entry {
                        source: resource.path(),
                        target: target.join(resource.file_name()),
                        resource: true,
                    });
                }
            }
        } else {
            entries.push(Entry {
                source: item.path(),
                target: staging.join(name),
                resource: false,
            });
        }
    }
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| -> Result<()> {
        let handles: Vec<_> = (0..entries.len().min(16))
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    while let Some(entry) = entries.get(next.fetch_add(1, Ordering::Relaxed)) {
                        copy_entry(source, entry)?;
                    }
                    Ok(())
                })
            })
            .collect();
        // Join every worker, including after failure, before staging cleanup.
        let results: Vec<_> = handles.into_iter().map(|handle| handle.join()).collect();
        for result in results {
            result.map_err(|_| anyhow::anyhow!("resource snapshot copy worker panicked"))??;
        }
        Ok(())
    })?;
    Ok(entries.len())
}

fn entry_type(entry: &std::fs::DirEntry) -> Result<std::fs::FileType> {
    // NFS readdir already carries this type. Path::is_dir would repeat a
    // network getattr for every entry in a mount with attribute caching off.
    let kind = entry.file_type()?;
    if kind.is_symlink()
        || (cfg!(windows)
            && opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(entry.path())?))
    {
        bail!(
            "agent resource entries cannot be symlinks: {}",
            entry.path().display()
        );
    }
    Ok(kind)
}

pub(super) fn selected(source: &Path, staging: &Path, names: &[String]) -> Result<usize> {
    let mut resources = std::collections::BTreeSet::new();
    let mut count = 0;
    for name in names.iter().collect::<std::collections::BTreeSet<_>>() {
        opencoder_core::agent::validate_agent_name(name).map_err(anyhow::Error::msg)?;
        let path = source.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.is_dir() && !opencoder_core::platform::fs::is_link(&metadata) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && opencoder_core::builtin_agents()
                        .iter()
                        .any(|agent| agent.name == *name) =>
            {
                continue
            }
            _ => bail!("selected agent card unavailable: {name}"),
        }
        let meta: AgentMeta =
            serde_json::from_slice(&super::files::read(&path.join("meta.json"))?)?;
        for (category, resource) in [
            ("prompts", meta.current.prompt),
            ("skills", meta.current.skills),
            ("tools", meta.current.tools),
            ("memory", meta.current.memory),
        ] {
            if let Some(resource) = resource {
                opencoder_core::agent::validate_resource_name(category, &resource)
                    .map_err(anyhow::Error::msg)?;
                resources.insert((category, resource));
            }
        }
        copy_entry(
            source,
            &Entry {
                source: path,
                target: staging.join(name),
                resource: false,
            },
        )?;
        count += 1;
    }
    for (category, name) in resources {
        let path = source.join(category).join(&name);
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || opencoder_core::platform::fs::is_link(&metadata) {
            bail!("selected Agent resource must be a real directory");
        }
        copy_entry(
            source,
            &Entry {
                source: path,
                target: staging.join(category).join(name),
                resource: true,
            },
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_entry(root: &Path, entry: &Entry) -> Result<()> {
    let meta_path = entry.source.join("meta.json");
    let raw = super::files::read(&meta_path)?;
    let version = if entry.resource {
        let meta: ResourceMeta = serde_json::from_slice(&raw)?;
        if meta.current == 0 {
            bail!("resource has no active version: {}", meta_path.display());
        }
        Some(format!("v{}", meta.current))
    } else {
        let _: AgentMeta = serde_json::from_slice(&raw)?;
        None
    };
    // This tree is private staging. Flush each populated directory once,
    // bottom-up, before publication instead of flushing empty directories
    // and their shared parents for every resource.
    std::fs::create_dir_all(&entry.target)?;
    std::fs::write(entry.target.join("meta.json"), raw)?;
    opencoder_core::platform::fs::sync_file(&entry.target.join("meta.json"))?;
    if let Some(version) = version {
        let path = entry.source.join(&version);
        if opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(&path)?) {
            bail!(
                "resource version root cannot be a symlink: {}",
                path.display()
            );
        }
        let original = path.canonicalize()?;
        if !original.starts_with(root) {
            bail!("resource version escaped configured resource root");
        }
        version_files(&original, &entry.target.join(version))?;
    }
    opencoder_core::platform::fs::sync_directory(&entry.target)?;
    Ok(())
}

pub(super) fn version_files(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("read resource version dir {}", source.display()))?
    {
        let entry = entry
            .with_context(|| format!("read resource version entry in {}", source.display()))?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink()
            || (cfg!(windows)
                && opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(&path)?))
        {
            bail!(
                "resource versions cannot contain symlinks: {}",
                path.display()
            );
        }
        if kind.is_dir() {
            version_files(&path, &destination.join(entry.file_name()))?;
        } else if kind.is_file() {
            let target = destination.join(entry.file_name());
            super::files::copy(&path, &target).with_context(|| {
                format!(
                    "copy resource file {} -> {}",
                    path.display(),
                    target.display()
                )
            })?;
            opencoder_core::platform::fs::sync_file(&target)
                .with_context(|| format!("sync copied resource file {}", target.display()))?;
        } else {
            bail!("resource versions must contain only regular files and directories");
        }
    }
    opencoder_core::platform::fs::sync_directory(destination)?;
    Ok(())
}
