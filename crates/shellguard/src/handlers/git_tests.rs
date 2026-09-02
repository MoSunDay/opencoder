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

/// #F6: the glued short form of an output flag names the same write
/// target as the spaced form and must reach the redirect pipeline.
#[test]
fn glued_short_output_flag_surfaces_the_target() {
    let targets_of = |args: &[&str]| -> Option<String> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        match GIT_HANDLER.classify(&HandlerContext::test("git", &owned)) {
            Classification::WithRedirects(_, refs) => refs.first().cloned(),
            _ => None,
        }
    };
    // The glued form used to be invisible to flag_path_value.
    assert_eq!(
        targets_of(&["archive", "-o/etc/x.tar", "HEAD"]),
        Some("/etc/x.tar".to_owned())
    );
    assert_eq!(
        targets_of(&["format-patch", "-1", "-o/etc/x"]),
        Some("/etc/x".to_owned())
    );
    // The spaced and `=` forms keep working in both directions.
    assert_eq!(
        targets_of(&["archive", "-o", "/tmp/ok.tar", "HEAD"]),
        Some("/tmp/ok.tar".to_owned())
    );
    assert_eq!(
        targets_of(&["archive", "--output=/tmp/ok.tar", "HEAD"]),
        Some("/tmp/ok.tar".to_owned())
    );
    assert_eq!(
        targets_of(&["format-patch", "--output-directory=/tmp/p", "-1"]),
        Some("/tmp/p".to_owned())
    );
    // No output flag: no redirect target, plain Allow.
    assert!(matches!(
        GIT_HANDLER.classify(&HandlerContext::test(
            "git",
            &[
                "archive".to_owned(),
                "--format=tar".to_owned(),
                "HEAD".to_owned()
            ]
        )),
        Classification::Allow(_)
    ));
}

/// #F11: `git branch --edit-description` launches $GIT_EDITOR — arbitrary
/// command execution through the environment — so it must Ask.
#[test]
fn branch_edit_description_asks() {
    for args in [
        vec!["branch", "--edit-description"],
        vec!["branch", "--edit-description", "main"],
    ] {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let result = GIT_HANDLER.classify(&HandlerContext::test("git", &owned));
        assert!(
            matches!(result, Classification::Ask(ref d) if d.contains("$GIT_EDITOR")),
            "git {args:?} must Ask on the editor flag, got {result:?}"
        );
    }
    // The editor flag cannot be laundered behind a plain read flag.
    let owned: Vec<String> = vec!["branch".into(), "-a".into(), "--edit-description".into()];
    assert!(matches!(
        GIT_HANDLER.classify(&HandlerContext::test("git", &owned)),
        Classification::Ask(_)
    ));
}

/// #F11 regression floor: pure listing forms of `git branch` stay allowed,
/// and the modify flags keep asking (including the `=` form of
/// `--set-upstream-to`, which `has_flag` alone missed).
#[test]
fn branch_listing_stays_allowed_and_modify_flags_stay_gated() {
    for args in [
        vec!["branch"],
        vec!["branch", "-a"],
        vec!["branch", "--list", "main"],
        vec!["branch", "-v"],
    ] {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(
                GIT_HANDLER.classify(&HandlerContext::test("git", &owned)),
                Classification::Allow(_)
            ),
            "git {args:?} is a read and must Allow"
        );
    }
    for args in [
        vec!["branch", "-d", "main"],
        vec!["branch", "--set-upstream-to=origin/main"],
        vec!["branch", "--set-upstream-to", "origin/main"],
        vec!["branch", "-m", "old", "new"],
    ] {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(
                GIT_HANDLER.classify(&HandlerContext::test("git", &owned)),
                Classification::Ask(_)
            ),
            "git {args:?} mutates and must Ask"
        );
    }
}
