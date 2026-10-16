//! Filesystem rules shared by HTTP storage and the read-only export.
use anyhow::{bail, ensure, Context, Result};
use opencoder_core::Config;
use opencoder_store::ProjectTodoRecord;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub(super) fn no_links(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "symbolic links are not allowed"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(crate) fn root(config: &Config, workdir: &Path, data: &Path) -> Result<PathBuf> {
    // The platform data root may itself be redirected (for example to another
    // disk). Resolve that trusted base before checking the dedicated subtree.
    let requested = match &config.project.todo_storage.files_dir {
        Some(path) => path.clone(),
        None => crate::ontology::resolved(&opencoder_core::data_dir_for(workdir))?
            .join("project/todo-files"),
    };
    no_links(&requested)?;
    let root = crate::ontology::resolved(&requested)?;
    let state = state_dir(&root);
    no_links(&state)?;
    let agents = opencoder_agents::default_opts_from_config(config).export_root;
    let binary = config
        .dag
        .binary_dir
        .clone()
        .unwrap_or_else(|| opencoder_core::data_dir_for(workdir).join("dag/binary"));
    for protected in [
        Some(agents),
        Some(binary),
        config.dag.workspace_dir.clone(),
        Some(config.ontology.files_for(workdir)),
    ]
    .into_iter()
    .flatten()
    {
        let protected = crate::ontology::resolved(&protected)?;
        for candidate in [&root, &state] {
            ensure!(
                !candidate.starts_with(&protected) && !protected.starts_with(candidate),
                "TODO files overlap another resource root"
            );
        }
    }
    let data = crate::ontology::resolved(data)?;
    ensure!(
        !data.starts_with(&root)
            && !data.starts_with(&state)
            && !root.starts_with(&data)
            && !state.starts_with(&data),
        "TODO export cannot contain Server databases"
    );
    ensure!(
        config.project.todo_storage.max_file_bytes > 0,
        "max_file_bytes must be positive"
    );
    Ok(root)
}

pub(super) fn state_dir(root: &Path) -> PathBuf {
    root.with_file_name(format!(
        "{}.state",
        root.file_name().unwrap_or_default().to_string_lossy()
    ))
}

pub(crate) fn initialize(config: &Config, workdir: &Path, data: &Path) -> Result<()> {
    let root = root(config, workdir, data)?;
    let data = crate::ontology::resolved(data)?;
    let binding = data.join("todo-storage.json");
    no_links(&binding)?;
    if binding.exists() {
        ensure!(
            root.is_dir() && state_dir(&root).is_dir(),
            "bound TODO storage directory is unavailable"
        );
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&binding)?)?;
        ensure!(saved["files_root"].as_str() == root.to_str(), "TODO files root differs from its saved binding; migrate files before changing configuration");
    }
    fs::create_dir_all(&root)?;
    let state = state_dir(&root);
    no_links(&state)?;
    fs::create_dir_all(&state)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            fs::metadata(&root)?.dev() == fs::metadata(&state)?.dev(),
            "TODO files and staging directory must be on the same filesystem"
        );
    }
    if !binding.exists() {
        let value = serde_json::to_vec(&serde_json::json!({"files_root":root}))?;
        // Publish without replacing a competing Server's binding.
        let temporary = data.join(format!("todo-storage-{}.tmp", ulid::Ulid::new()));
        fs::write(&temporary, value)?;
        fs::File::open(&temporary)?.sync_all()?;
        let result = fs::hard_link(&temporary, &binding);
        fs::remove_file(&temporary)?;
        if let Err(error) = result {
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
            let saved: serde_json::Value = serde_json::from_slice(&fs::read(&binding)?)?;
            ensure!(
                saved["files_root"].as_str() == root.to_str(),
                "TODO root binding conflict"
            );
        }
        opencoder_core::platform::fs::sync_directory(&data)?;
    }
    Ok(())
}

pub(super) fn relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 4096 && !path.contains(['\\', '\0', '\r', '\n']),
        "invalid file path"
    );
    ensure!(
        path.split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && part.len() <= 255),
        "invalid file path"
    );
    Ok(())
}

pub(super) fn directory(todo: &ProjectTodoRecord) -> Result<String> {
    relative(&todo.id)?;
    ensure!(!todo.id.contains('/'), "invalid TODO id");
    let date = chrono::DateTime::from_timestamp_millis(todo.created_at)
        .context("invalid TODO creation date")?;
    Ok(format!("{}/{}", date.format("%Y-%m-%d"), todo.id))
}

pub(super) fn path(root: &Path, relative_path: &str) -> Result<PathBuf> {
    relative(relative_path)?;
    let path = root.join(relative_path);
    no_links(&path)?;
    Ok(path)
}

pub(super) async fn lock(root: &Path, id: &str) -> Result<fs::File> {
    relative(id)?;
    ensure!(!id.contains('/'), "invalid TODO id");
    lock_file(root, &format!("{id}.lock"), false).await
}

pub(super) async fn global_lock(root: &Path) -> Result<fs::File> {
    lock_file(root, ".global.lock", true).await
}

async fn lock_file(root: &Path, name: &str, shared: bool) -> Result<fs::File> {
    let directory = state_dir(root);
    no_links(&directory)?;
    fs::create_dir_all(&directory)?;
    let file_path = directory.join(name);
    no_links(&file_path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(file_path)?;
    loop {
        let result = if shared {
            fs2::FileExt::try_lock_shared(&file)
        } else {
            fs2::FileExt::try_lock_exclusive(&file)
        };
        match result {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[derive(Serialize)]
pub(super) struct Entry {
    pub path: String,
    pub bytes: u64,
    pub modified_at: u128,
}

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    no_links(root)?;
    if !root.try_exists()? {
        return Ok(vec![]);
    }
    let mut result = vec![];
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(directory)? {
            let item = item?;
            let meta = fs::symlink_metadata(item.path())?;
            if meta.is_dir() {
                pending.push(item.path());
            } else if meta.is_file() {
                result.push(Entry {
                    path: item
                        .path()
                        .strip_prefix(root)?
                        .to_str()
                        .context("non-UTF8 file path")?
                        .replace('\\', "/"),
                    bytes: meta.len(),
                    modified_at: meta.modified()?.duration_since(UNIX_EPOCH)?.as_millis(),
                });
            } else {
                bail!("TODO storage contains a symbolic link or non-regular file");
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

pub(super) fn open_file(path: &Path) -> Result<fs::File> {
    no_links(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "not a regular file");
    Ok(file)
}

/// Flush the complete newly-created directory chain before acknowledging an upload.
pub(super) fn sync_parents(directory: &Path, root: &Path) -> Result<()> {
    ensure!(
        directory.starts_with(root),
        "directory is outside TODO export"
    );
    for parent in directory.ancestors() {
        opencoder_core::platform::fs::sync_directory(parent)?;
        if Some(parent) == root.parent() {
            break;
        }
    }
    Ok(())
}

/// Never recursively unlink files, even if an out-of-band writer adds one.
pub(super) fn remove_empty_tree(root: &Path) -> Result<()> {
    no_links(root)?;
    if !root.try_exists()? {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut directories = vec![];
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(&directory)? {
            let item = item?;
            ensure!(
                item.file_type()?.is_dir(),
                "TODO storage is no longer empty"
            );
            pending.push(item.path());
        }
        directories.push(directory);
    }
    for directory in directories.into_iter().rev() {
        fs::remove_dir(directory)?;
    }
    Ok(())
}
