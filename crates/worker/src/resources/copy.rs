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
            opencoder_core::share_fs::durable_create_dir_all(&target)?;
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
    if kind.is_symlink() {
        bail!(
            "agent resource entries cannot be symlinks: {}",
            entry.path().display()
        );
    }
    Ok(kind)
}

fn copy_entry(root: &Path, entry: &Entry) -> Result<()> {
    let meta_path = entry.source.join("meta.json");
    let raw = std::fs::read(&meta_path)?;
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
    opencoder_core::share_fs::durable_create_dir_all(&entry.target)?;
    std::fs::write(entry.target.join("meta.json"), raw)?;
    std::fs::File::open(entry.target.join("meta.json"))?.sync_all()?;
    std::fs::File::open(&entry.target)?.sync_all()?;
    if let Some(version) = version {
        let path = entry.source.join(&version);
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
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
    Ok(())
}

pub(super) fn version_files(source: &Path, destination: &Path) -> Result<()> {
    opencoder_core::share_fs::durable_create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("read resource version dir {}", source.display()))?
    {
        let entry = entry
            .with_context(|| format!("read resource version entry in {}", source.display()))?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!(
                "resource versions cannot contain symlinks: {}",
                path.display()
            );
        }
        if kind.is_dir() {
            version_files(&path, &destination.join(entry.file_name()))?;
        } else {
            let target = destination.join(entry.file_name());
            std::fs::copy(&path, &target).with_context(|| {
                format!(
                    "copy resource file {} -> {}",
                    path.display(),
                    target.display()
                )
            })?;
            std::fs::File::open(&target)
                .with_context(|| format!("reopen copied resource file {}", target.display()))?
                .sync_all()?;
        }
    }
    std::fs::File::open(destination)?.sync_all()?;
    Ok(())
}
