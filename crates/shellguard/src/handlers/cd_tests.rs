//! Ported from rippy (MIT) https://github.com/mpecan/rippy
//!
//! Unit tests for the `cd`/`pushd` handler (separate file to stay under the
//! 400-line limit; assertions compacted through the helpers below).

use std::path::{Path, PathBuf};

use super::*;

fn classify_in(cwd: &str, cmd: &str, args: &[&str], scopes: &[&str]) -> Classification {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let scopes: Vec<PathBuf> = scopes.iter().map(PathBuf::from).collect();
    CD_HANDLER.classify(&HandlerContext {
        working_directory: Path::new(cwd),
        safe_scopes: &scopes,
        ..HandlerContext::test(cmd, &args)
    })
}

/// Classify with the standard test cwd `/project` and no declared scopes.
fn classify(cmd: &str, args: &[&str]) -> Classification {
    classify_in("/project", cmd, args, &[])
}

fn is_allow(c: &Classification) -> bool {
    matches!(c, Classification::Allow(_))
}

fn is_ask(c: &Classification) -> bool {
    matches!(c, Classification::Ask(_))
}

/// Plan mode maps `Allow && writes_state` to a block, so a read-only `cd`
/// must carry non-writing provenance.
#[test]
fn cd_allow_is_not_a_state_write() {
    for args in [
        vec!["src"],
        vec![".."],
        vec!["/etc"],
        vec!["/tmp"],
        vec!["-"],
        vec!["-P", "src"],
        vec!["--", "/var"],
    ] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let c = CD_HANDLER.classify(&HandlerContext {
            working_directory: Path::new("/project"),
            safe_scopes: &[],
            ..HandlerContext::test("cd", &args)
        });
        // `clippy::panic` is denied workspace-wide (regression gate), so the
        // allow-or-fail decision is a single assert instead of a match/panic.
        assert!(
            matches!(c, Classification::Allow(ref reason) if !reason.writes_state()),
            "cd {args:?} must allow without a state write, got: {c:?}"
        );
    }
}

// cd with no args

#[test]
fn cd_no_args_asks() {
    assert!(is_ask(&classify("cd", &[])));
}

// cd -

#[test]
fn cd_dash_allows() {
    assert!(is_allow(&classify("cd", &["-"])));
}

// cd ~

#[test]
fn cd_tilde_asks() {
    assert!(is_ask(&classify("cd", &["~"])));
}

#[test]
fn cd_tilde_subdir_asks() {
    assert!(is_ask(&classify("cd", &["~/documents"])));
}

// variable expansion

#[test]
fn cd_variable_asks() {
    assert!(is_ask(&classify("cd", &["$HOME"])));
}

#[test]
fn cd_command_substitution_asks() {
    assert!(is_ask(&classify("cd", &["$(pwd)"])));
}

#[test]
fn cd_backtick_asks() {
    assert!(is_ask(&classify("cd", &["`pwd`"])));
}

// relative paths within project

#[test]
fn cd_relative_subdir_allows() {
    // cd writes no state; the resolvable destination keeps later relative
    // operands judgeable (the analyzer re-aims its cwd).
    assert!(is_allow(&classify("cd", &["src"])));
}

#[test]
fn cd_relative_nested_allows() {
    assert!(is_allow(&classify("cd", &["src/handlers"])));
}

#[test]
fn cd_dot_allows() {
    // cd writes no state; `.` is statically resolvable
    assert!(is_allow(&classify("cd", &["."])));
}

#[test]
fn cd_dotdot_from_subdir_allows() {
    // going up writes nothing; a later write would be judged against the
    // resolved parent (the analyzer re-aims its cwd)
    assert!(is_allow(&classify_in("/project/src", "cd", &[".."], &[])));
}

// relative paths escaping project

#[test]
fn cd_dotdot_from_root_allows() {
    assert!(is_allow(&classify("cd", &[".."])));
}

#[test]
fn cd_relative_escape_allows() {
    // the cd itself is read-only; a write after it is still judged against
    // the resolved destination
    assert!(is_allow(&classify("cd", &["../../etc"])));
}

// absolute paths

#[test]
fn cd_absolute_within_project_allows() {
    assert!(is_allow(&classify("cd", &["/project/src"])));
}

#[test]
fn cd_absolute_outside_project_allows() {
    assert!(is_allow(&classify("cd", &["/etc"])));
}

// safe directories

#[test]
fn cd_tmp_allows() {
    assert!(is_allow(&classify("cd", &["/tmp"])));
}

#[test]
fn cd_tmp_subdir_allows() {
    assert!(is_allow(&classify("cd", &["/tmp/build"])));
}

#[test]
fn cd_var_tmp_allows() {
    // outside the release set, but cd itself is a no-op on the filesystem
    assert!(is_allow(&classify("cd", &["/var/tmp"])));
}

// config-allowed directories

#[test]
fn cd_to_config_allowed_dir_allows() {
    assert!(is_allow(&classify_in(
        "/project",
        "cd",
        &["/opt/repos/other-project"],
        &["/opt/repos"]
    )));
}

#[test]
fn cd_to_config_allowed_exact_allows() {
    assert!(is_allow(&classify_in(
        "/project",
        "cd",
        &["/opt/repos"],
        &["/opt/repos"]
    )));
}

#[test]
fn cd_outside_config_allowed_allows() {
    // cd is read-only regardless of scopes; pushd outside scopes still asks
    assert!(is_allow(&classify_in(
        "/project",
        "cd",
        &["/etc"],
        &["/opt/repos"]
    )));
}

#[test]
fn cd_relative_resolves_into_allowed_parent() {
    // CWD is within an allowed parent — relative cd that stays within is ok
    assert!(is_allow(&classify_in(
        "/opt/repos/project-a",
        "cd",
        &["../project-b"],
        &["/opt/repos"]
    )));
}

#[test]
fn cd_multiple_allowed_dirs() {
    assert!(is_allow(&classify_in(
        "/project",
        "cd",
        &["/opt/repos/foo"],
        &["/opt/repos", "/home/user/work"]
    )));
    assert!(is_allow(&classify_in(
        "/project",
        "cd",
        &["/home/user/work/bar"],
        &["/opt/repos", "/home/user/work"]
    )));
    assert!(is_allow(&classify_in(
        "/project",
        "cd",
        &["/home/user/personal"],
        &["/opt/repos", "/home/user/work"]
    )));
}

// leading option flags shifting the destination

#[test]
fn cd_dash_p_outside_scope_allows() {
    assert!(is_allow(&classify("cd", &["-P", "/etc"])));
}

#[test]
fn cd_dash_p_within_cwd_allows() {
    assert!(is_allow(&classify("cd", &["-P", "src"])));
}

#[test]
fn cd_double_dash_outside_scope_allows() {
    assert!(is_allow(&classify("cd", &["--", "/etc"])));
}

#[test]
fn cd_double_dash_within_cwd_allows() {
    assert!(is_allow(&classify("cd", &["--", "src"])));
}

#[test]
fn cd_unknown_flag_asks() {
    assert!(is_ask(&classify("cd", &["--unknown", "/tmp"])));
}

// pushd

#[test]
fn pushd_within_project_asks() {
    // sandbox: cwd is not a release set
    assert!(is_ask(&classify("pushd", &["src"])));
}

#[test]
fn pushd_outside_project_asks() {
    assert!(is_ask(&classify("pushd", &["/etc"])));
}

#[test]
fn pushd_tmp_allows() {
    assert!(is_allow(&classify("pushd", &["/tmp"])));
}

#[test]
fn pushd_to_config_allowed_allows() {
    assert!(is_allow(&classify_in(
        "/project",
        "pushd",
        &["/opt/repos/other"],
        &["/opt/repos"]
    )));
}

// popd

#[test]
fn popd_asks() {
    assert!(is_ask(&classify("popd", &[])));
}

// remote mode

#[test]
fn cd_remote_asks() {
    let args = ["src".to_string()];
    let ctx = HandlerContext {
        remote: true,
        ..HandlerContext::test("cd", &args)
    };
    assert!(is_ask(&CD_HANDLER.classify(&ctx)));
}

// normalize_path

#[test]
fn normalize_resolves_dotdot() {
    assert_eq!(
        normalize_path(Path::new("/a/b/../c")),
        PathBuf::from("/a/c")
    );
}

#[test]
fn normalize_resolves_dot() {
    assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
}

#[test]
fn normalize_multiple_dotdot() {
    assert_eq!(
        normalize_path(Path::new("/a/b/c/../../d")),
        PathBuf::from("/a/d")
    );
}
