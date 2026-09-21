//! A Runtime owns its global skill bytes, including embedded release resources.
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub(super) fn pinned_root() -> Option<PathBuf> {
    ROOT.get().cloned()
}

// Execution-scoped skill root: operator executions freeze their own skill
// packs into `<execution home>/.opencoder/skills`, and the sessions they
// run discover skills from THAT root only — never the node-level snapshot
// (which mirrors the interactive user's `~/.opencoder/skills`) and never
// the daemon user's home. Scoped exactly like `agent::scope`.
tokio::task_local! {
    static EXECUTION_ROOT: Option<PathBuf>;
}

pub fn execution_root() -> Option<PathBuf> {
    EXECUTION_ROOT.try_with(Clone::clone).ok().flatten()
}

pub async fn with_execution<F: std::future::Future>(root: Option<PathBuf>, future: F) -> F::Output {
    EXECUTION_ROOT.scope(root, future).await
}

/// Called once before Runtime sessions start. Keep user skills and dependency
/// opt-ins, then seed this binary's embedded packs into the private snapshot.
/// Later startups reuse exactly these bytes; a newer CLI or Runtime may update
/// the user's global skills without changing an older execution's resources.
pub fn pin_runtime_skills(data: &Path, shared: Option<&Path>) -> io::Result<()> {
    fs::create_dir_all(data)?;
    let root = data.canonicalize()?.join("global-skills");
    if let Some(current) = ROOT.get() {
        return if *current == root {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Runtime skill root is already pinned",
            ))
        };
    }
    freeze(shared, &root)?;
    ROOT.set(root)
        .map_err(|_| io::Error::other("Runtime skill root initialized concurrently"))?;
    Ok(())
}

fn freeze(source: Option<&Path>, root: &Path) -> io::Result<()> {
    if root.exists() {
        if root.canonicalize()? != root || !root.join(".runtime-ready").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Runtime skill snapshot is not complete or canonical",
            ));
        }
        return Ok(());
    }
    let parent = root
        .parent()
        .ok_or_else(|| io::Error::other("Runtime skills require a parent"))?;
    let stage = parent.join(format!(".skills-stage-{}", ulid::Ulid::new()));
    fs::create_dir(&stage)?;
    // Failed snapshots stay as evidence. Never remove user directories or a
    // database that a user skill might contain while retrying a release.
    if let Some(source) = source.filter(|p| p.exists()) {
        copy_tree(source, &stage, &mut HashSet::new())?;
    }
    super::seed_builtin_skills_in(&stage)?;
    super::seed_dep_gated_skills_in(&stage)?;
    fs::write(stage.join(".runtime-ready"), b"1\n")?;
    sync_tree(&stage)?;
    fs::rename(&stage, root)?;
    fs::File::open(parent)?.sync_all()
}

fn copy_tree(source: &Path, target: &Path, ancestors: &mut HashSet<PathBuf>) -> io::Result<()> {
    let canonical = source.canonicalize()?;
    if target.canonicalize()?.starts_with(&canonical) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "skill source contains its Runtime snapshot",
        ));
    }
    if !ancestors.insert(canonical.clone()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cycle in global skill resources",
        ));
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source = entry.path();
        let target = target.join(entry.file_name());
        let metadata = fs::metadata(&source)?;
        if metadata.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(&source, &target, ancestors)?;
        } else if metadata.is_file() {
            fs::copy(source, target)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "global skill contains a non-file resource",
            ));
        }
    }
    ancestors.remove(&canonical);
    Ok(())
}

fn sync_tree(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            sync_tree(&path)?;
        } else {
            fs::File::open(path)?.sync_all()?;
        }
    }
    fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_releases_keep_skill_bytes_when_shared_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shared");
        fs::create_dir_all(source.join("custom")).unwrap();
        fs::write(source.join("custom/SKILL.md"), "first release").unwrap();
        let first = dir.path().join("r1");
        let second = dir.path().join("r2");
        freeze(Some(&source), &first).unwrap();
        fs::write(source.join("custom/SKILL.md"), "second release").unwrap();
        freeze(Some(&source), &second).unwrap();
        freeze(Some(&source), &first).unwrap();
        assert_eq!(
            fs::read_to_string(first.join("custom/SKILL.md")).unwrap(),
            "first release"
        );
        assert_eq!(
            fs::read_to_string(second.join("custom/SKILL.md")).unwrap(),
            "second release"
        );
        fs::create_dir(dir.path().join("partial")).unwrap();
        assert!(freeze(None, &dir.path().join("partial")).is_err());
    }
}
