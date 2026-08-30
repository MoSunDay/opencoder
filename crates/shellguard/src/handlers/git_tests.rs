//! Ported from rippy (MIT) https://github.com/mpecan/rippy
//!
//! Unit tests for the `git` handler (kept in a separate file to stay under the
//! 400-line limit).

use super::*;

    // Pure subcommand->decision cases (safe/ask subcommands, branch -d, stash list,
    // tag) and ABSOLUTE out-of-scope repo redirects are covered by the catalog
    // (rippy's command catalog (not ported)). The tests below need injected state
    // the catalog cannot reach: a cwd-relative/in-project target (fixed `/tmp` cwd)
    // or a non-empty `safe_scopes`.
    /// A guarded entry naming a subcommand outside `SAFE_SUBCOMMANDS` never
    /// runs and never renders a catalog row.
    #[test]
    fn every_guarded_subcommand_is_a_safe_subcommand() {
        for (name, guard, _) in GUARDED_SAFE_SUBCOMMANDS {
            assert!(
                SAFE_SUBCOMMANDS.contains(name),
                "{name} is guarded but not in SAFE_SUBCOMMANDS"
            );
            assert!(!guard.is_empty(), "{name} declares an empty guard");
        }
    }

    #[test]
    fn global_flags_skipped() {
        let args = vec!["-C".into(), "/tmp".into(), "status".into()];
        let result = GIT_HANDLER.classify(&HandlerContext::test("git", &args));
        assert!(matches!(result, Classification::Allow(_)));
    }

    #[test]
    fn git_dir_eq_within_declared_scope_allows() {
        let allowed = vec![std::path::PathBuf::from("/opt/repos")];
        let args = vec!["--git-dir=/opt/repos/other/.git".into(), "log".into()];
        let ctx = HandlerContext {
            safe_scopes: &allowed,
            ..HandlerContext::test("git", &args)
        };
        assert!(matches!(
            GIT_HANDLER.classify(&ctx),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn git_dir_eq_within_project_allows() {
        // `=` form pointing inside the project directory still resolves normally.
        let args = vec!["--git-dir=/tmp/sub/.git".into(), "status".into()];
        let result = GIT_HANDLER.classify(&HandlerContext::test("git", &args));
        assert!(matches!(result, Classification::Allow(_)));
    }

    #[test]
    fn dash_c_within_project_allows() {
        let args = vec!["-C".into(), "/tmp/subdir".into(), "status".into()];
        let result = GIT_HANDLER.classify(&HandlerContext::test("git", &args));
        assert!(matches!(result, Classification::Allow(_)));
    }

    #[test]
    fn dash_c_relative_allows() {
        let args = vec!["-C".into(), "subdir".into(), "status".into()];
        let result = GIT_HANDLER.classify(&HandlerContext::test("git", &args));
        assert!(matches!(result, Classification::Allow(_)));
    }

    // Rejected-widening guard: read-only `git -C <undeclared> log` must still
    // Ask. An arbitrary repo's `.git/config` (pager/alias) can run code even on
    // a "read" subcommand, so scope opt-in is required — see #134.
    #[test]
    fn dash_c_undeclared_read_only_still_asks() {
        let args = vec!["-C".into(), "/opt/other-repo".into(), "log".into()];
        let result = GIT_HANDLER.classify(&HandlerContext::test("git", &args));
        assert!(matches!(result, Classification::Ask(_)));
    }

    // Within a declared scope, the same read-only command is allowed (parity).
    #[test]
    fn dash_c_declared_scope_read_only_allows() {
        let allowed = vec![std::path::PathBuf::from("/opt/repos")];
        let args = vec!["-C".into(), "/opt/repos/other".into(), "log".into()];
        let ctx = HandlerContext {
            safe_scopes: &allowed,
            ..HandlerContext::test("git", &args)
        };
        assert!(matches!(
            GIT_HANDLER.classify(&ctx),
            Classification::Allow(_)
        ));
    }

    // Within a declared scope, a writing subcommand still Asks (write guard).
    #[test]
    fn dash_c_declared_scope_write_still_asks() {
        let allowed = vec![std::path::PathBuf::from("/opt/repos")];
        let args = vec!["-C".into(), "/opt/repos/other".into(), "push".into()];
        let ctx = HandlerContext {
            safe_scopes: &allowed,
            ..HandlerContext::test("git", &args)
        };
        assert!(matches!(GIT_HANDLER.classify(&ctx), Classification::Ask(_)));
    }

    #[test]
    fn dash_c_config_allowed() {
        let allowed = vec![std::path::PathBuf::from("/opt/repos")];
        let args = vec!["-C".into(), "/opt/repos/other".into(), "status".into()];
        let ctx = HandlerContext {
            safe_scopes: &allowed,
            ..HandlerContext::test("git", &args)
        };
        assert!(matches!(
            GIT_HANDLER.classify(&ctx),
            Classification::Allow(_)
        ));
}
