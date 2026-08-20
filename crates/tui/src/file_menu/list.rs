//! Workdir file listing for the `@` picker — a capped, gitignore-aware walk
//! (`ignore::WalkBuilder`, the same engine as the `search` tool).

use std::path::Path;

use ignore::WalkBuilder;

/// Hard cap on collected entries: bounds the sync walk on huge repos.
pub const MAX_ENTRIES: usize = 2000;
/// Directory depth cap (root = depth 0): keeps deep `target/`-style trees
/// from dominating the list.
pub const MAX_DEPTH: usize = 8;

/// One pickable path. `rel` is `/`-separated, workdir-relative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub rel: String,
    pub is_dir: bool,
}

/// List workdir files+dirs (hidden, `.git`, gitignored and binary-unfriendly
/// entries skipped), lexicographically sorted, capped at `MAX_ENTRIES`.
pub fn collect_entries(workdir: &Path) -> Vec<FileEntry> {
    collect_entries_with(workdir, MAX_ENTRIES, MAX_DEPTH)
}

/// Parameterized core for tests (small caps without creating thousands of
/// files). Stops as soon as `max` entries are collected.
pub fn collect_entries_with(workdir: &Path, max: usize, depth: usize) -> Vec<FileEntry> {
    let mut out: Vec<FileEntry> = Vec::new();
    if max == 0 {
        return out;
    }
    let mut wb = WalkBuilder::new(workdir);
    // hidden(true) also skips `.git`; parents(false) ignores gitignore files
    // above the workdir; require_git(false) honors .gitignore even outside
    // a git repo.
    wb.hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .ignore(true)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .max_depth(Some(depth));
    for e in wb.build() {
        if out.len() >= max {
            break;
        }
        let e = match e {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ft = match e.file_type() {
            Some(t) if t.is_file() || t.is_dir() => t,
            _ => continue,
        };
        let rel = match e.path().strip_prefix(workdir) {
            Ok(p) => p,
            Err(_) => continue,
        };
        // Depth-0 root entry (the workdir itself).
        if rel.as_os_str().is_empty() {
            continue;
        }
        out.push(FileEntry {
            rel: rel.to_string_lossy().replace('\\', "/"),
            is_dir: ft.is_dir(),
        });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.dedup_by(|a, b| a.rel == b.rel);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_and_dirs_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("a/c.rs"), "x").unwrap();
        let entries = collect_entries(dir.path());
        let rels: Vec<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(rels, vec!["a", "a/c.rs", "b.txt"]);
        assert!(entries[0].is_dir && !entries[1].is_dir);
    }

    #[test]
    fn skips_hidden_and_gitignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.txt"), "x").unwrap();
        std::fs::write(dir.path().join(".hidden"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "x").unwrap();
        std::fs::write(dir.path().join("gen.txt"), "x").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "gen.txt\n").unwrap();
        let rels: Vec<String> = collect_entries(dir.path())
            .into_iter()
            .map(|e| e.rel)
            .collect();
        assert_eq!(
            rels,
            vec!["keep.txt".to_string()],
            "hidden + gitignored all skipped"
        );
        assert!(!rels.contains(&".hidden".to_string()));
        assert!(
            !rels.contains(&"gen.txt".to_string()),
            "gitignored entry must be skipped"
        );
    }

    #[test]
    fn caps_entries_at_max() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let entries = collect_entries_with(dir.path(), 3, 8);
        assert_eq!(entries.len(), 3, "collection stops at the cap");
    }

    #[test]
    fn depth_limit_excludes_deep_paths() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("x.txt"), "x").unwrap();
        // depth 2: "a" (1) and "a/b" (2) list; "a/b/c" (3) and its file are cut.
        let rels: Vec<String> = collect_entries_with(dir.path(), 100, 2)
            .into_iter()
            .map(|e| e.rel)
            .collect();
        assert_eq!(rels, vec!["a", "a/b"]);
    }

    #[test]
    fn missing_workdir_yields_empty() {
        assert!(collect_entries(Path::new("/nonexistent-opencoder-test")).is_empty());
    }
}
