//! Path-scope helpers: logical normalization and the sandbox release set.
//!
//! Trimmed derivative of rippy's `handlers/mod.rs` scope section (MIT,
//! https://github.com/mpecan/rippy). Sandbox delta: rippy's
//! `is_within_scope` treated the working directory as writable
//! (`path.starts_with(cwd) || safe_dir`); that clause is deliberately NOT
//! ported -- in sandbox mode the project directory is never a release set, so
//! only [`RELEASE_DIRECTORIES`] approve path-based operations.

use std::path::{Path, PathBuf};

/// Directories released by the sandbox: writes under them are allowed.
///
/// `/dev/null` covers device redirects; `/tmp` is the scratch area. There is no
/// `/var/tmp` or `/private/tmp` here (unlike rippy's defaults) -- the release
/// set is exactly what the sandbox policy declares.
pub(crate) const RELEASE_DIRECTORIES: &[&str] = &["/dev/null", "/tmp"];

/// Logical path normalization: resolve `.` and `..` components without
/// filesystem access (the target directory may not exist yet).
#[must_use]
pub(crate) fn normalize_path(path: &Path) -> std::path::PathBuf {
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

/// Check if a resolved, normalized path is within the sandbox release set
/// (plus any extra declared scopes).
///
/// Matching uses `Path::starts_with`, which respects path-component
/// boundaries: a scope of `/tmp` matches `/tmp/x` but NOT the sibling
/// `/tmpx`. Do not replace this with string `starts_with`.
pub(crate) fn is_within_safe_dir(path: &Path, extra_scopes: &[std::path::PathBuf]) -> bool {
    if extra_scopes.iter().any(|d| path.starts_with(d)) {
        return true;
    }
    is_within_release_dir(path)
}

/// Check if a normalized path is within one of the built-in
/// [`RELEASE_DIRECTORIES`] (`/dev/null`, `/tmp`).
///
/// Callers that auto-approve writes apply extra symlink hardening on top of
/// this check (the release dirs are world-writable).
#[must_use]
pub(crate) fn is_within_release_dir(path: &Path) -> bool {
    RELEASE_DIRECTORIES.iter().any(|safe| path.starts_with(safe))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> std::path::PathBuf {
        normalize_path(Path::new(s))
    }

    #[test]
    fn release_set_matches_component_boundaries() {
        assert!(is_within_release_dir(&p("/tmp/a.log")));
        assert!(is_within_release_dir(&p("/tmp")));
        assert!(is_within_release_dir(&p("/dev/null")));
        // No sibling-prefix or `..` escape.
        assert!(!is_within_release_dir(&p("/tmpx")));
        assert!(!is_within_release_dir(&p("/tmpnope/x")));
        assert!(!is_within_release_dir(&p("/tmp/../etc/passwd")));
        assert!(!is_within_release_dir(&p("/var/x")));
    }

    #[test]
    fn extra_scopes_extend_the_release_set() {
        let extra = [std::path::PathBuf::from("/scratch")];
        assert!(is_within_safe_dir(&p("/scratch/a"), &extra));
        assert!(is_within_safe_dir(&p("/tmp/a"), &extra));
        assert!(!is_within_safe_dir(&p("/etc/a"), &extra));
    }

    #[test]
    fn normalize_collapses_dot_and_dotdot() {
        assert_eq!(p("./a/./b"), std::path::PathBuf::from("a/b"));
        assert_eq!(p("/tmp/a/../b"), std::path::PathBuf::from("/tmp/b"));
        assert_eq!(p("/a/b/../../c"), std::path::PathBuf::from("/c"));
    }
}

/// Resolve symlinks by canonicalizing the deepest ancestor of `path` that
/// exists on disk, then re-appending the non-existing tail components.
///
/// Unlike [`crate::handlers::normalize_path`] (purely logical), this follows
/// symlinks, so a redirect target routed through a planted symlink resolves to
/// its real location. The tail is preserved because a write target usually does
/// not exist yet. Falls back to the input path when nothing can be
/// canonicalized.
pub(crate) fn canonicalize_existing_ancestor(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            let mut result = real;
            result.extend(tail.iter().rev());
            return result;
        }
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                ancestor = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Resolve a command operand and check it against the release set, hardened
/// against planted symlinks.
///
/// Relative operands resolve against `working_directory`. The logical path
/// must be inside the release set (plus any declared extra scopes), *and* its
/// symlink-resolved real path (deepest existing ancestor canonicalized) must
/// still be inside the release dirs — otherwise a link planted under `/tmp`
/// pointing elsewhere would launder the write.
pub(crate) fn operand_in_release(
    operand: &str,
    working_directory: &Path,
    extra_scopes: &[std::path::PathBuf],
) -> bool {
    let raw = Path::new(operand);
    let resolved = if raw.is_absolute() {
        normalize_path(raw)
    } else {
        normalize_path(&working_directory.join(raw))
    };
    if !is_within_safe_dir(&resolved, extra_scopes) {
        return false;
    }
    is_within_release_dir(&canonicalize_existing_ancestor(&resolved))
}

#[cfg(test)]
mod operand_scope_tests {
    use super::*;

    #[test]
    fn missing_and_existing_released_paths_both_pass() {
        assert!(operand_in_release("/tmp/definitely-missing-file", Path::new("/project"), &[]));
        assert!(operand_in_release("/dev/null", Path::new("/project"), &[]));
    }

    #[test]
    fn relative_operands_resolve_against_the_working_directory() {
        assert!(!operand_in_release("x", Path::new("/project"), &[]));
        assert!(operand_in_release("x", Path::new("/tmp"), &[]));
    }
}
