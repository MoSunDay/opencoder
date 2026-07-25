use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Hard ceiling on the number of filesystem entries visited. Backstops the
/// canonical-path cycle guard for cases it cannot catch (e.g. permission
/// errors or pathological non-symlink fan-out). Mirrors grep.rs::walk.
const MAX_VISITED: u32 = 50_000;
/// Maximum number of result paths returned. Matches the previous `.take(500)`.
const RESULT_CAP: usize = 500;
/// Directories pruned before recursion — kept in sync with grep.rs::walk.
const PRUNE: &[&str] = &[".git", "node_modules", "target", "dist", ".next", ".cache"];

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Fast file pattern matching. Returns file paths matching the glob pattern (e.g. \"**/*.rs\")."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "pattern".into(),
            json::prop_str("Glob pattern, e.g. \"src/**/*.rs\"."),
        );
        props.insert("path".into(), json::prop_str("Optional base directory."));
        json::object_schema(Value::Object(props), &["pattern"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", base.display(), pattern)
        };
        // Compile the full pattern once and match each visited path against it,
        // instead of letting glob::glob() drive an unbounded recursive descent.
        // The glob crate follows symlinks when deciding directory-ness and has
        // no cycle guard, so a self-referencing symlink loop hangs it.
        let compiled = match glob::Pattern::new(&full_pattern) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::err(format!("invalid glob: {e}"))),
        };
        let opts = glob::MatchOptions {
            // `require_literal_separator = true` keeps glob semantics intact:
            // `*` cannot cross `/`. This matches how glob::glob() descends and
            // preserves parity with the previous implementation's results.
            require_literal_separator: true,
            case_sensitive: true,
            require_literal_leading_dot: false,
        };
        // A pattern whose final component is `**` matches directories only in
        // the glob crate; gate matches on is_dir to preserve that.
        let dir_only = is_doublestar_terminal(&full_pattern);
        let mut out: Vec<String> = Vec::new();
        if is_literal(&full_pattern) {
            // Fully-literal pattern (no wildcards): the glob crate returns the
            // exact path if it exists, else nothing. Handle directly instead of
            // walking, since `literal_root` would resolve to the file itself.
            let p = Path::new(&full_pattern);
            if p.exists() {
                out.push(p.display().to_string());
            }
        } else {
            let root = literal_root(&full_pattern, &base);
            let mut visited = 0u32;
            let mut seen: HashSet<PathBuf> = HashSet::new();
            walk(
                &root, &compiled, &opts, dir_only, &mut out, &mut visited, &mut seen,
            );
        }
        out.sort();
        if out.is_empty() {
            return Ok(ToolOutput::ok("no matches"));
        }
        let joined = out
            .iter()
            .take(RESULT_CAP)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        Ok(opencoder_core::tool::truncate_output(joined, ctx.max_output))
    }
}

/// Recursive directory walker that breaks symlink cycles via canonical-path
/// dedup, mirroring grep.rs::walk. Each directory is canonicalized and recorded
/// in `seen`; a self-referencing symlink (`loop -> .`) resolves to an ancestor's
/// canonical path and is pruned instead of recursing forever.
fn walk(
    dir: &Path,
    pattern: &glob::Pattern,
    opts: &glob::MatchOptions,
    dir_only: bool,
    out: &mut Vec<String>,
    visited: &mut u32,
    seen: &mut HashSet<PathBuf>,
) {
    if *visited > MAX_VISITED || out.len() >= RESULT_CAP {
        return;
    }
    let canon = match dir.canonicalize() {
        Ok(c) => c,
        Err(_) => return,
    };
    if !seen.insert(canon) {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *visited > MAX_VISITED || out.len() >= RESULT_CAP {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = path.is_dir();
        *visited += 1;
        if (!dir_only || is_dir) && pattern.matches_path_with(&path, *opts) {
            out.push(path.display().to_string());
        }
        if is_dir && !PRUNE.contains(&name.as_str()) {
            walk(&path, pattern, opts, dir_only, out, visited, seen);
        }
    }
}

/// True when the pattern's final path component is exactly `**`. The glob
/// crate only yields directories for such patterns, so matches are gated on
/// `is_dir` to preserve result parity with `glob::glob()`.
fn is_doublestar_terminal(pattern: &str) -> bool {
    Path::new(pattern)
        .file_name()
        .map(|f| f == std::ffi::OsStr::new("**"))
        .unwrap_or(false)
}

/// True when the pattern contains no glob metacharacters (`*`, `?`, `[`).
/// Such patterns are exact paths; the glob crate returns the path itself if it
/// exists (file or directory), which the walker cannot express (it only ever
/// yields descendants of its root).
fn is_literal(pattern: &str) -> bool {
    !pattern.chars().any(|c| matches!(c, '*' | '?' | '['))
}

/// Longest literal (non-glob) prefix of `pattern`, used as the descent root so
/// unrelated subtrees are not scanned. For `src/**/*.rs` the root is `<base>/src`;
/// for `*.rs` (no literal dir) it falls back to `base`.
fn literal_root(pattern: &str, base: &Path) -> PathBuf {
    let mut root = PathBuf::new();
    for comp in Path::new(pattern).components() {
        let s = comp.as_os_str().to_string_lossy();
        if s.contains('*') || s.contains('?') || s.contains('[') {
            break;
        }
        root.push(comp);
    }
    if root.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_root_stops_at_first_glob_component() {
        let base = Path::new("/work");
        assert_eq!(literal_root("/work/src/**/*.rs", base), PathBuf::from("/work/src"));
        assert_eq!(literal_root("/work/a/b/c.rs", base), PathBuf::from("/work/a/b/c.rs"));
        assert_eq!(literal_root("/work/*.rs", base), PathBuf::from("/work"));
        assert_eq!(literal_root("*.rs", base), PathBuf::from("/work"));
    }

    #[test]
    fn is_doublestar_terminal_detects_trailing_recursive() {
        assert!(is_doublestar_terminal("/w/**"));
        assert!(is_doublestar_terminal("/w/sub/**"));
        assert!(!is_doublestar_terminal("/w/**/*.rs"));
        assert!(!is_doublestar_terminal("/w/*.rs"));
        assert!(is_doublestar_terminal("/w/sub/**/"));
    }

    #[test]
    fn is_literal_detects_wildcards() {
        assert!(is_literal("/work/a.rs"));
        assert!(is_literal("/work/sub/b.rs"));
        assert!(!is_literal("/work/*.rs"));
        assert!(!is_literal("/work/**/*.rs"));
        assert!(!is_literal("/work/[abc].rs"));
        assert!(!is_literal("/work/a?b"));
    }
}
