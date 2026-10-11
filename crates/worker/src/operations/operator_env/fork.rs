use super::*;

/// A fork takes the parent's frozen configuration and skills, never today's
/// node pool. The new execution owns its HOME and an empty working directory.
pub(crate) fn fork_environment(
    layout: &DirectoryLayout,
    kind: ExecutionKind,
    parent: &str,
    id: &str,
    version: Option<&serde_json::Value>,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let Some((source, _)) = resolve(layout, kind, parent, version)? else {
        return Ok(None);
    };
    let (home, workspace) = create_directories(layout, kind, id)?;
    write_snapshot(
        &snapshot_path(&home),
        &std::fs::read_to_string(snapshot_path(&source))?,
    )?;
    copy_directory(
        &source.join(".opencoder/skills"),
        &home.join(".opencoder/skills"),
    )?;
    // Codex fork needs the original thread in its private home. Copy its
    // state so resuming the child cannot write to the parent's HOME.
    let codex = source.join(".codex");
    if codex.exists() {
        copy_directory(&codex, &home.join(".codex"))?;
    }
    Ok(Some((home, workspace)))
}

fn copy_directory(source: &Path, target: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    ensure!(
        metadata.is_dir() && !opencoder_core::platform::fs::is_link(&metadata),
        "operator fork source must be a directory, not a link"
    );
    #[cfg(not(windows))]
    opencoder_core::share_fs::durable_create_dir_all(target)?;
    #[cfg(windows)]
    opencoder_core::platform::fs::ensure_private_directory(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            !opencoder_core::platform::fs::is_link(&metadata),
            "operator fork cannot copy a link: {}",
            path.display()
        );
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_directory(&path, &destination)?;
        } else {
            ensure!(
                metadata.is_file(),
                "operator fork can only copy regular files"
            );
            std::fs::copy(&path, &destination)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_copies_frozen_state_without_sharing_writes() {
        let temp = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(temp.path().to_path_buf(), None).unwrap();
        let kind = ExecutionKind::Operator;
        let (parent, _) = materialize(&layout, kind, "parent", &Default::default())
            .unwrap()
            .unwrap();
        let pack = ".opencoder/skills/frozen.md";
        std::fs::write(parent.join(pack), "original skill").unwrap();
        std::fs::create_dir(parent.join(".codex")).unwrap();
        std::fs::write(parent.join(".codex/thread"), "original thread").unwrap();
        let version = serde_json::json!(1);
        let (home, workspace) = fork_environment(&layout, kind, "parent", "child", Some(&version))
            .unwrap()
            .unwrap();
        assert_ne!(home, parent);
        assert_eq!(std::fs::read_dir(workspace).unwrap().count(), 0);
        assert_eq!(std::fs::read(home.join(pack)).unwrap(), b"original skill");
        assert_eq!(
            std::fs::read(home.join(".codex/thread")).unwrap(),
            b"original thread"
        );
        std::fs::write(home.join(pack), "child skill").unwrap();
        assert_eq!(std::fs::read(parent.join(pack)).unwrap(), b"original skill");
        assert!(opencoder_core::platform::fs::private_access(&snapshot_path(&home)).unwrap());
        assert!(resolve(&layout, kind, "child", Some(&version))
            .unwrap()
            .is_some());
        assert!(fork_environment(&layout, kind, "parent", "legacy", None)
            .unwrap()
            .is_none());
    }
}
