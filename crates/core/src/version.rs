//! Build-time version metadata: package SemVer + git commit.
//!
//! The git commit is resolved by `build.rs` (short + full hash, plus a dirty
//! flag) and surfaced here as compile-time constants. `--version`, the server
//! banner and `/api/health` all read from here, so the commit id travels with
//! every version surface. Non-git builds fall back to "unknown".

/// Package version (SemVer only), e.g. "0.1.0". Shown by `-V`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git commit at build time, e.g. "125b34c", or "unknown".
pub const GIT_COMMIT: &str = match option_env!("OPENCODER_GIT_COMMIT") {
    Some(v) => v,
    None => "unknown",
};

/// Full git commit at build time, or "unknown".
pub const GIT_COMMIT_FULL: &str = match option_env!("OPENCODER_GIT_COMMIT_FULL") {
    Some(v) => v,
    None => "unknown",
};

/// True if the working tree had uncommitted changes at build time. (A runtime
/// check rather than a const because `&str` equality is not const-stable yet.)
pub fn is_dirty() -> bool {
    matches!(option_env!("OPENCODER_GIT_DIRTY"), Some("1"))
}

/// Long version string baked at build time, e.g. "0.1.0 (125b34c)" or
/// "0.1.0 (125b34c-dirty)". Used by `--version` and server surfaces.
pub const VERSION_LONG: &str = match option_env!("OPENCODER_VERSION_LONG") {
    Some(v) => v,
    None => VERSION,
};

/// Pure format helper: assemble a version string from parts. Independently
/// unit-tested, and asserted to agree with the build-time baked constant.
pub fn format_version(version: &str, commit: &str, dirty: bool) -> String {
    if dirty {
        format!("{version} ({commit}-dirty)")
    } else {
        format!("{version} ({commit})")
    }
}

/// The long version string (the build-time constant).
pub fn long_version() -> &'static str {
    VERSION_LONG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_clean() {
        assert_eq!(format_version("0.1.0", "125b34c", false), "0.1.0 (125b34c)");
    }

    #[test]
    fn format_version_dirty_appends_marker() {
        assert_eq!(
            format_version("0.1.0", "125b34c", true),
            "0.1.0 (125b34c-dirty)"
        );
    }

    #[test]
    fn format_version_unknown_commit_is_honest() {
        assert_eq!(format_version("0.1.0", "unknown", false), "0.1.0 (unknown)");
    }

    /// The build-time baked constant must agree with the pure format helper fed
    /// the same captured inputs. Guards against `build.rs`/format drift.
    #[test]
    fn baked_long_version_matches_format_contract() {
        assert_eq!(
            VERSION_LONG,
            format_version(VERSION, GIT_COMMIT, is_dirty())
        );
    }

    /// The whole point of this module: the version must carry the commit id.
    #[test]
    fn long_version_carries_commit_id() {
        assert!(
            VERSION_LONG.contains(GIT_COMMIT),
            "VERSION_LONG={VERSION_LONG} missing commit {GIT_COMMIT}"
        );
        assert!(VERSION_LONG.contains('(') && VERSION_LONG.contains(')'));
    }
}
