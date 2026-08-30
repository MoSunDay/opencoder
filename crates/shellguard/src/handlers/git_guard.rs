//! `git` global-flag guards: repo redirection (`-C`, `--git-dir`, `--work-tree`)
//! and `-c`/`--config-env` config-key overrides, checked before subcommand
//! dispatch.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use std::path::Path;

use super::{Classification, HandlerContext, is_within_safe_dir, normalize_path};

/// Config keys `-c`/`--config-env` may set without asking. Deliberately an
/// allowlist, not a denylist: any key not listed here (core.pager,
/// core.sshCommand, alias.*, uploadpack.packObjectsHook, protocol.*.allow, ...)
/// can run arbitrary commands via git's config, so unknown keys fail closed.
pub(crate) const SAFE_CONFIG_KEYS: &[&str] = &[
    "user.name",
    "user.email",
    "color.ui",
    "core.autocrlf",
    "core.quotepath",
    "init.defaultbranch",
    "pull.rebase",
    "advice.detachedhead",
];

/// Flags that redirect git to a different repository location. `-C` takes a
/// separate value argument; `--git-dir`/`--work-tree` accept either a separate
/// value or an attached `=` form (`--git-dir=PATH`).
const REPO_PATH_FLAGS: &[&str] = &["-C", "--git-dir", "--work-tree"];

/// Repo-redirect flags that also support the attached `--flag=PATH` form.
const REPO_PATH_EQ_FLAGS: &[&str] = &["--git-dir", "--work-tree"];

/// If git is invoked with -C, --git-dir, or --work-tree pointing outside
/// the allowed scope, return Ask. Otherwise return None to continue
/// normal classification. Both the separated (`--git-dir PATH`) and attached
/// (`--git-dir=PATH`) forms are checked so the redirect cannot be smuggled past
/// the scope guard.
pub(crate) fn check_repo_path_flags(ctx: &HandlerContext) -> Option<Classification> {
    let mut i = 0;
    while i < ctx.args.len() {
        let arg = ctx.args[i].as_str();
        if let Some((flag, value)) = repo_redirect_target(arg, ctx.args.get(i + 1)) {
            if let Some(verdict) = repo_flag_out_of_scope(flag, value, ctx) {
                return Some(verdict);
            }
            // The attached `=` form consumes one arg; the separated form two.
            i += if arg.contains('=') { 1 } else { 2 };
            continue;
        }
        i += 1;
    }
    None
}

/// If `arg` is a repo-redirect flag, return the `(flag-label, target-path)` it
/// points at — handling both `--git-dir PATH` (value in `next`) and the
/// attached `--git-dir=PATH` form.
pub(crate) fn repo_redirect_target<'a>(arg: &'a str, next: Option<&'a String>) -> Option<(&'a str, &'a str)> {
    if REPO_PATH_FLAGS.contains(&arg) {
        return next.map(|v| (arg, v.as_str()));
    }
    for flag in REPO_PATH_EQ_FLAGS {
        if let Some(value) = arg
            .strip_prefix(flag)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some((flag, value));
        }
    }
    None
}

/// Return `Ask` when a repo-redirect flag targets a path outside the allowed
/// scope, otherwise `None`.
///
/// Sandbox policy: the working directory is NOT a writable scope, so only a
/// declared safe scope (or a built-in release dir) passes.
pub(crate) fn repo_flag_out_of_scope(
    flag: &str,
    value: &str,
    ctx: &HandlerContext,
) -> Option<Classification> {
    let resolved = if Path::new(value).is_absolute() {
        normalize_path(Path::new(value))
    } else {
        normalize_path(&ctx.working_directory.join(value))
    };
    if is_within_safe_dir(&resolved, ctx.safe_scopes) {
        None
    } else {
        Some(Classification::Ask(format!(
            "git {flag} targets outside allowed scope ({value})"
        )))
    }
}

/// Scrutinize `-c key=value` and `--config-env` overrides before subcommand
/// dispatch: `extract_subcommand` skips them to find the subcommand, but their
/// key must be checked against `SAFE_CONFIG_KEYS` first, since these can carry
/// RCE-bearing keys (core.pager, core.sshCommand, uploadpack.packObjectsHook, ...).
pub(crate) fn check_config_overrides(args: &[String]) -> Option<Classification> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "-c" {
            if let Some(kv) = args.get(i + 1) {
                if let Some(verdict) = check_config_kv(kv) {
                    return Some(verdict);
                }
            }
            i += 2;
            continue;
        }
        if let Some(kv) = arg.strip_prefix("--config-env=") {
            if let Some(verdict) = check_config_kv(kv) {
                return Some(verdict);
            }
            i += 1;
            continue;
        }
        if arg == "--config-env" {
            if let Some(kv) = args.get(i + 1) {
                if let Some(verdict) = check_config_kv(kv) {
                    return Some(verdict);
                }
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    None
}

/// `None` when `kv`'s key (case-insensitive, up to the first `=`) is on the
/// safe allowlist; `Some(Ask)` otherwise.
pub(crate) fn check_config_kv(kv: &str) -> Option<Classification> {
    let key = kv.split('=').next().unwrap_or(kv).to_lowercase();
    if SAFE_CONFIG_KEYS.contains(&key.as_str()) {
        None
    } else {
        Some(Classification::Ask(format!(
            "git -c {kv} is not on the safe config allowlist"
        )))
    }
}
