//! Pure file-change validation and merging. Bytes and Unix permissions survive edits.
use crate::io::invalid_input;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub const MAX_TOTAL_BYTES: usize = 1536 * 1024;
pub const MAX_FILES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub resource: Option<String>,
    pub version: u32,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub content_b64: String,
    /// Only ordinary permission bits, never setuid/setgid/sticky.
    pub mode: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub bytes: Vec<u8>,
    pub mode: u32,
}
impl From<&FileEntry> for FileChange {
    fn from(file: &FileEntry) -> Self {
        Self {
            path: file.path.clone(),
            content_b64: B64.encode(&file.bytes),
            mode: Some(file.mode),
        }
    }
}

#[derive(Deserialize)]
pub struct SaveRequest {
    pub baseline: Baseline,
    #[serde(default)]
    pub files: Vec<FileChange>,
    #[serde(default)]
    pub removed: Vec<String>,
}
#[derive(Deserialize)]
pub struct RestoreRequest {
    pub baseline: Baseline,
    pub version: u32,
}

pub fn validate_path(path: &str) -> io::Result<()> {
    if path.is_empty()
        || path.contains(['\\', '\0'])
        || path.split('/').count() > 64
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid_input(format!("unsafe file path: {path}")));
    }
    Ok(())
}

pub fn validate_files(cat: &str, files: &[FileEntry]) -> io::Result<()> {
    if !opencoder_core::agent::AGENT_CATEGORIES.contains(&cat) {
        return Err(invalid_input("unknown resource category"));
    }
    if files.len() > MAX_FILES
        || files.iter().map(|f| f.bytes.len()).sum::<usize>() > MAX_TOTAL_BYTES
    {
        return Err(invalid_input("resource exceeds 1.5 MiB or 4096 files"));
    }
    let paths: BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
    if paths.len() != files.len() {
        return Err(invalid_input("duplicate file path"));
    }
    for file in files {
        validate_path(&file.path)?;
        if file.mode & !0o777 != 0 {
            return Err(invalid_input("invalid file permission bits"));
        }
        let valid = match cat {
            "prompts" => matches!(file.path.as_str(), "soul.md" | "how.md" | "output.md"),
            // Memory is directory-shaped: any safe relative path (the
            // reader aggregates every `*.md` under the version dir).
            "memory" => true,
            "skills" => file.path.split_once('/').map_or_else(
                || file.path.ends_with(".md"),
                |(skill, _)| paths.contains(format!("{skill}/SKILL.md").as_str()),
            ),
            _ => true,
        };
        if !valid {
            return Err(invalid_input(format!("invalid {cat} file: {}", file.path)));
        }
        let parts: Vec<_> = file.path.split('/').collect();
        for end in 1..parts.len() {
            if paths.contains(parts[..end].join("/").as_str()) {
                return Err(invalid_input("file and directory paths overlap"));
            }
        }
        // Markdown stays text-only for prompts and memory (prompts only
        // admits `.md` paths anyway); non-markdown memory files (dumps,
        // json sidecars) may be binary.
        if matches!(cat, "prompts" | "memory")
            && file.path.ends_with(".md")
            && (std::str::from_utf8(&file.bytes).is_err() || file.bytes.contains(&0))
        {
            return Err(invalid_input("markdown must be UTF-8"));
        }
    }
    if cat == "prompts"
        && !files
            .iter()
            .any(|f| std::str::from_utf8(&f.bytes).is_ok_and(|s| !s.trim().is_empty()))
    {
        return Err(invalid_input(
            "Prompt requires at least one non-empty section",
        ));
    }
    Ok(())
}

pub fn merge_files(
    cat: &str,
    original: &[FileEntry],
    changes: &[FileChange],
    removed: &[String],
) -> io::Result<Vec<FileEntry>> {
    if changes.len() + removed.len() > MAX_FILES * 2 {
        return Err(invalid_input("too many file changes"));
    }
    let mut files: BTreeMap<_, _> = original
        .iter()
        .map(|f| (f.path.clone(), f.clone()))
        .collect();
    let mut touched = BTreeSet::new();
    for path in removed {
        validate_path(path)?;
        if !touched.insert(path.as_str()) {
            return Err(invalid_input("duplicate file change"));
        }
        if files.remove(path).is_none() {
            return Err(invalid_input(format!("cannot remove missing file: {path}")));
        }
    }
    let mut total = 0;
    for change in changes {
        validate_path(&change.path)?;
        if !touched.insert(change.path.as_str()) {
            return Err(invalid_input("duplicate file change"));
        }
        // Limit before allocating the decoded buffer too.
        if change.content_b64.len() > MAX_TOTAL_BYTES * 4 / 3 + 4 {
            return Err(invalid_input("decoded payload exceeds 1.5 MiB"));
        }
        let bytes = B64
            .decode(&change.content_b64)
            .map_err(|e| invalid_input(format!("invalid base64: {e}")))?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err(invalid_input("decoded payload exceeds 1.5 MiB"));
        }
        let mode = change
            .mode
            .unwrap_or_else(|| files.get(&change.path).map_or(0o600, |f| f.mode));
        files.insert(
            change.path.clone(),
            FileEntry {
                path: change.path.clone(),
                bytes,
                mode,
            },
        );
    }
    let files: Vec<_> = files.into_values().collect();
    validate_files(cat, &files)?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_preserves_bytes_modes_and_rejects_ambiguous_changes() {
        let files = vec![FileEntry {
            path: "run".into(),
            bytes: vec![0, 255],
            mode: 0o751,
        }];
        let changes = vec![FileChange {
            path: "run".into(),
            content_b64: B64.encode(b"new"),
            mode: None,
        }];
        assert_eq!(merge_files("tools", &files, &[], &[]).unwrap(), files);
        assert_eq!(
            merge_files("tools", &files, &changes, &[]).unwrap()[0].mode,
            0o751
        );
        assert!(merge_files("tools", &files, &changes, &["run".into()]).is_err());
        for path in ["../x", "/x", "x//y", "x/./y", "x\\y", "x\0"] {
            assert!(validate_path(path).is_err());
        }
        assert!(validate_files("prompts", &[]).is_err());
        assert!(validate_files(
            "skills",
            &[FileEntry {
                path: "s/data".into(),
                ..files[0].clone()
            }]
        )
        .is_err());
    }
}
