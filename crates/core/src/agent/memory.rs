//! Shared memory pool aggregation for file-based custom agents.
//!
//! A memory reference resolves to a whole version directory that may hold
//! many markdown files in nested folders (the write side accepts any safe
//! relative path). The read side aggregates deterministically: every
//! non-hidden `*.md` under the version dir is read, trimmed, and the
//! bodies are joined by one blank line in relative-path lexicographic
//! order. A pool holding a single `memory.md` therefore renders
//! byte-for-byte like the pre-T3 single-file read.

use std::path::Path;

/// Aggregated memory cap. Mirrors `AGENTS_MD_MAX_BYTES` in
/// `crates/session/src/prompt.rs` (200 KiB) — that constant is
/// `pub(crate)` to the session crate, so core keeps its own equal value.
const MEMORY_MAX_BYTES: usize = 200 * 1024;

/// Aggregate every markdown file under a memory version `dir` into the
/// `# Memory` section body. Returns `None` when no readable `*.md` exists
/// (callers then append no memory section at all). Unreadable or
/// non-UTF-8 files are skipped with a `debug` log — the pool degrades,
/// it never fails the whole agent.
pub(crate) fn section_body(dir: &Path) -> Option<String> {
    let mut paths: Vec<String> = Vec::new();
    collect_markdown(dir, dir, &mut paths);
    paths.sort();
    let bodies: Vec<String> = paths
        .iter()
        .filter_map(|rel| match std::fs::read_to_string(dir.join(rel)) {
            Ok(body) => Some(body.trim().to_string()),
            Err(e) => {
                tracing::debug!(path = %rel, error = %e,
                    "memory pool: skipping unreadable markdown file");
                None
            }
        })
        .map(|body| body.trim().to_string())
        .collect();
    if bodies.is_empty() {
        return None;
    }
    Some(cap(bodies.join("\n\n")))
}

/// Walk `dir` recursively collecting relative paths of every non-hidden
/// `*.md` regular file. Symlinks are neither descended nor collected
/// (the write path rejects them; out-of-tree content never leaks in).
fn collect_markdown(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue; // hidden files and hidden subtrees are skipped
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect_markdown(root, &path, out);
        } else if kind.is_file() && name.ends_with(".md") {
            out.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
}

/// Cap an aggregated body at [`MEMORY_MAX_BYTES`], char-boundary safe:
/// past the limit keep the first `MEMORY_MAX_BYTES` bytes and append a
/// marker carrying the original size (same style as
/// `cap_instructions` in `crates/session/src/prompt.rs`).
fn cap(body: String) -> String {
    if body.len() <= MEMORY_MAX_BYTES {
        return body;
    }
    let mut end = MEMORY_MAX_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[memory truncated: original size {} bytes exceeds {}KB limit]",
        &body[..end],
        body.len(),
        MEMORY_MAX_BYTES / 1024
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_passes_short_bodies_through_and_marks_long_ones() {
        assert_eq!(cap("short".into()), "short");
        let long = "a".repeat(MEMORY_MAX_BYTES + 7);
        let capped = cap(long.clone());
        assert!(capped.starts_with(&"a".repeat(MEMORY_MAX_BYTES)));
        assert!(capped.ends_with(&format!(
            "[memory truncated: original size {} bytes exceeds 200KB limit]",
            long.len()
        )));
    }

    #[test]
    fn section_body_skips_unreadable_files_and_degrades() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "alpha\n").unwrap();
        // Invalid UTF-8 fails `read_to_string` exactly like an unreadable
        // file: the entry is logged at debug level and dropped, the rest
        // of the pool still aggregates.
        std::fs::write(dir.path().join("bad.md"), [0xFF, 0xFE]).unwrap();
        assert_eq!(section_body(dir.path()).unwrap(), "alpha");
        // A pool holding no readable markdown appends no section at all.
        let only_bad = tempfile::tempdir().unwrap();
        std::fs::write(only_bad.path().join("bad.md"), [0xFF]).unwrap();
        assert_eq!(section_body(only_bad.path()), None);
    }

    #[test]
    fn cap_truncates_on_a_char_boundary() {
        // Two 3-byte chars straddle the limit: the cut backs up to the
        // ASCII run instead of splitting a character.
        let body = format!("{}{}", "a".repeat(MEMORY_MAX_BYTES - 1), "漢漢");
        let capped = cap(body);
        let head = capped
            .split_once("\n\n[memory truncated:")
            .map(|(h, _)| h)
            .unwrap();
        assert_eq!(head.len(), MEMORY_MAX_BYTES - 1);
        assert!(head.is_char_boundary(head.len()));
    }
}
