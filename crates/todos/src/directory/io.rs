use super::{decode, encode, Files};
use crate::WorkflowSpec;
use anyhow::{bail, Context, Result};
use opencoder_core::share_fs::{atomic_write, durable_create_dir_all};
use std::path::Path;

/// Accept legacy JSON input explicitly; directory format errors never fall back.
pub fn load(path: &Path) -> Result<WorkflowSpec> {
    if path.is_file() {
        return crate::parse_spec(&std::fs::read_to_string(path)?);
    }
    let files = read_files(path)?;
    decode(&files).map(|(spec, _)| spec).map_err(|errors| {
        anyhow::anyhow!(
            "{}",
            errors
                .iter()
                .map(|e| format!("{}:{}:{}: {}", e.path, e.line, e.column, e.message))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

pub fn read_files(root: &Path) -> Result<Files> {
    let mut files = Files::new();
    read_tree(root, root, &mut files, 0)?;
    if !files.contains_key("workflow.json") {
        if let Some(legacy) = files.get("context.json") {
            let spec = crate::parse_spec(legacy)?;
            let binding: serde_json::Value = files
                .get("env.json")
                .map(|s| serde_json::from_str(s))
                .transpose()?
                .unwrap_or_default();
            let env = binding.get("env").and_then(serde_json::Value::as_str);
            let mut converted = encode(&spec, env)?;
            // Preserve unrecognized files so validation exposes them to the editor.
            for (path, text) in files {
                if path != "context.json" && path != "env.json" {
                    converted.insert(path, text);
                }
            }
            return Ok(converted);
        }
    }
    Ok(files)
}

fn read_tree(root: &Path, path: &Path, files: &mut Files, depth: usize) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("read {}", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!("TODO files cannot be symlinks: {}", path.display());
    }
    if depth > 3 {
        bail!("TODO directory is too deep: {}", path.display());
    }
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            read_tree(root, &entry?.path(), files, depth + 1)?;
        }
    } else if meta.is_file() {
        let name = path
            .strip_prefix(root)?
            .to_str()
            .context("non-UTF8 TODO file name")?;
        files.insert(
            name.into(),
            std::fs::read_to_string(path).with_context(|| format!("read {name}"))?,
        );
    } else {
        bail!("TODO entry must be a regular file: {}", path.display());
    }
    Ok(())
}

/// Publish a complete new directory. Existing versions can never be overwritten.
pub fn write_new(root: &Path, files: &Files) -> Result<()> {
    decode(files).map_err(|errors| {
        anyhow::anyhow!("{}", serde_json::to_string(&errors).unwrap_or_default())
    })?;
    if root.exists() {
        bail!("TODO version already exists: {}", root.display());
    }
    let parent = root.parent().context("TODO directory has no parent")?;
    durable_create_dir_all(parent)?;
    let staging = parent.join(format!(".todo-staging-{}", ulid::Ulid::new()));
    let result = (|| {
        durable_create_dir_all(&staging)?;
        for (path, content) in files {
            atomic_write(&staging.join(path), content.as_bytes())?;
        }
        // All production publishers hold the template lock while choosing the version.
        if root.exists() {
            bail!("TODO version already exists");
        }
        std::fs::rename(&staging, root)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() && staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    result
}
