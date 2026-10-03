use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::error::AppError;

pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct TextStore {
    root: PathBuf,
}

impl TextStore {
    pub async fn initialize(root: PathBuf) -> Result<Self, AppError> {
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()?.join(root)
        };
        let existing = root
            .ancestors()
            .find(|path| path.exists())
            .map(Path::to_path_buf);
        tokio::fs::create_dir_all(&root).await?;
        for path in root.ancestors() {
            tokio::fs::File::open(path).await?.sync_all().await?;
            if existing.as_deref() == Some(path) {
                break;
            }
        }
        let canonical = tokio::fs::canonicalize(root).await?;
        Ok(Self { root: canonical })
    }

    pub fn relative_path(
        env: &str,
        entity: Uuid,
        attribute: i64,
        revision: i64,
        format: &str,
    ) -> Result<PathBuf, AppError> {
        if !matches!(format, "md" | "html") {
            return Err(AppError::invalid("text format must be md or html"));
        }
        if revision < 1 {
            return Err(AppError::invalid("text revision must be positive"));
        }
        Ok(PathBuf::from(env)
            .join("entities")
            .join(entity.to_string())
            .join("attributes")
            .join(attribute.to_string())
            .join(format!("{revision}-{}.{format}", Uuid::new_v4())))
    }

    pub async fn write_immutable(
        &self,
        relative: &Path,
        content: &[u8],
    ) -> Result<String, AppError> {
        if content.len() > MAX_TEXT_BYTES {
            return Err(AppError::invalid("text content exceeds 4 MiB"));
        }
        validate_relative(relative)?;
        let target = self.root.join(relative);
        let expected = hex::encode(Sha256::digest(content));
        if let Ok(existing) = self.read(relative).await {
            return if hex::encode(Sha256::digest(existing)) == expected {
                Ok(expected)
            } else {
                Err(AppError::Conflict(
                    "immutable text path already contains different content".into(),
                ))
            };
        }
        let parent = target
            .parent()
            .ok_or_else(|| AppError::invalid("text path has no parent"))?;
        let mut checked = self.root.clone();
        for component in relative
            .parent()
            .ok_or_else(|| AppError::invalid("missing text parent"))?
            .components()
        {
            checked.push(component);
            match tokio::fs::create_dir(&checked).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let metadata = tokio::fs::symlink_metadata(&checked).await?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::Forbidden);
            }
        }
        let canonical_parent = tokio::fs::canonicalize(parent).await?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(AppError::Forbidden);
        }
        // Exclusive creation is atomic on NFS too. A failed/partial file belongs
        // to its failed revision and is never replaced by a later writer.
        match tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
            .await
        {
            Ok(mut file) => {
                file.write_all(content).await?;
                file.sync_all().await?;
                for ancestor in parent.ancestors() {
                    tokio::fs::File::open(ancestor).await?.sync_all().await?;
                    if ancestor == self.root {
                        break;
                    }
                }
                Ok(expected)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.read(relative).await?;
                if hex::encode(Sha256::digest(existing)) == expected {
                    Ok(expected)
                } else {
                    Err(AppError::Conflict(
                        "immutable text path already contains different content".into(),
                    ))
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn read(&self, relative: &Path) -> Result<Vec<u8>, AppError> {
        validate_relative(relative)?;
        let target = self.root.join(relative);
        let canonical = tokio::fs::canonicalize(&target).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound
            } else {
                error.into()
            }
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(AppError::Forbidden);
        }
        Ok(tokio::fs::read(canonical).await?)
    }

    pub fn nfs_relative_path(path: &str) -> Result<PathBuf, AppError> {
        let candidate = Path::new(path.trim());
        if candidate.as_os_str().is_empty()
            || candidate.is_absolute()
            || candidate
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(AppError::invalid(
                "NFS path must be a controlled relative path",
            ));
        }
        Ok(candidate.to_path_buf())
    }

    pub async fn read_nfs(&self, relative: &Path) -> Result<Vec<u8>, AppError> {
        let relative = Self::nfs_relative_path(&relative.to_string_lossy())?;
        let target = self.root.join(relative);
        let canonical = tokio::fs::canonicalize(&target).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound
            } else {
                error.into()
            }
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(AppError::Forbidden);
        }
        let metadata = tokio::fs::metadata(&canonical).await?;
        if !metadata.is_file() || metadata.len() > MAX_TEXT_BYTES as u64 {
            return Err(AppError::invalid(
                "NFS reference must be a text file of at most 4 MiB",
            ));
        }
        Ok(tokio::fs::read(canonical).await?)
    }

    pub async fn ping(&self) -> Result<(), AppError> {
        let metadata = tokio::fs::metadata(&self.root).await?;
        if !metadata.is_dir() {
            return Err(AppError::dependency("text storage root is not a directory"));
        }
        Ok(())
    }
}

fn validate_relative(path: &Path) -> Result<(), AppError> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::invalid("text path is invalid"));
    }
    match path.extension().and_then(|v| v.to_str()) {
        Some("md" | "html") => Ok(()),
        _ => Err(AppError::invalid("text path must end in .md or .html")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn immutable_write_is_idempotent_and_rejects_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let store = TextStore::initialize(temp.path().to_path_buf())
            .await
            .unwrap();
        let path = Path::new("debug/entities/a/attributes/1/1.md");
        assert_eq!(
            store.write_immutable(path, b"hello").await.unwrap(),
            store.write_immutable(path, b"hello").await.unwrap()
        );
        assert!(store.write_immutable(path, b"changed").await.is_err());
        assert_eq!(store.read(path).await.unwrap(), b"hello");
    }
    #[test]
    fn generated_paths_reject_unsafe_formats() {
        assert!(TextStore::relative_path("debug", Uuid::new_v4(), 1, 1, "txt").is_err());
        assert!(validate_relative(Path::new("../escape.md")).is_err());
        assert!(TextStore::nfs_relative_path("/etc/passwd").is_err());
        assert!(TextStore::nfs_relative_path("../escape").is_err());
        assert!(TextStore::nfs_relative_path("debug/source.txt").is_ok());
    }
    #[tokio::test]
    async fn concurrent_writers_never_replace_the_winner() {
        let temp = tempfile::tempdir().unwrap();
        let store = TextStore::initialize(temp.path().to_path_buf())
            .await
            .unwrap();
        let path = Path::new("race/1.md");
        let (a, b) = tokio::join!(
            store.write_immutable(path, b"first"),
            store.write_immutable(path, b"second")
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let expected = if a.is_ok() {
            b"first".as_slice()
        } else {
            b"second".as_slice()
        };
        assert_eq!(store.read(path).await.unwrap(), expected);
    }
}
