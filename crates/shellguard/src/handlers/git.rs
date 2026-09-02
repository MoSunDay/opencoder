//! `git` command handler: safe/ask subcommand split with guarded read-only
//! subcommands (`--ext-diff`, `-o/--output`, `--open-files-in-pager`, `--extcmd`,
//! URL-like fetch operands) and repo/config override checks (in `git_guard.rs`).
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::git_guard::{check_config_overrides, check_repo_path_flags};
use super::{git_subcommands, has_flag, positional_args, Classification, Handler, HandlerContext};
use crate::verdict::AllowReason;

pub(crate) static GIT_HANDLER: GitHandler = GitHandler;

pub(crate) struct GitHandler;

const SAFE_SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "show",
    "diff",
    "blame",
    "annotate",
    "shortlog",
    "describe",
    "rev-parse",
    "rev-list",
    "reflog",
    "whatchanged",
    "diff-tree",
    "diff-files",
    "diff-index",
    "range-diff",
    "format-patch",
    "difftool",
    "grep",
    "ls-files",
    "ls-tree",
    "ls-remote",
    "cat-file",
    "verify-commit",
    "verify-tag",
    "name-rev",
    "merge-base",
    "show-ref",
    "show-branch",
    "check-ignore",
    "cherry",
    "for-each-ref",
    "count-objects",
    "fsck",
    "var",
    "request-pull",
    "archive",
    "fetch",
    "version",
    "help",
];

const ASK_SUBCOMMANDS: &[&str] = &[
    "commit",
    "add",
    "rm",
    "mv",
    "restore",
    "reset",
    "revert",
    "push",
    "pull",
    "checkout",
    "switch",
    "merge",
    "rebase",
    "cherry-pick",
    "clean",
    "gc",
    "prune",
    "filter-branch",
    "filter-repo",
    "submodule",
    "worktree",
    "init",
    "clone",
    "am",
    "apply",
];

/// Global flags that take a value argument (skip both flag and value).
const GLOBAL_VALUE_FLAGS: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--config-env",
];

/// Global flags that are standalone (skip just the flag).
const GLOBAL_FLAGS: &[&str] = &[
    "--no-pager",
    "--bare",
    "--no-replace-objects",
    "--literal-pathspecs",
    "--glob-pathspecs",
    "--noglob-pathspecs",
    "--icase-pathspecs",
    "--no-optional-locks",
    "--paginate",
    "-p",
];

impl Handler for GitHandler {
    fn commands(&self) -> &[&str] {
        &["git"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        // Check if -C, --git-dir, or --work-tree points outside allowed scope
        if let Some(verdict) = check_repo_path_flags(ctx) {
            return verdict;
        }

        if let Some(verdict) = check_config_overrides(ctx.args) {
            return verdict;
        }

        let (sub, sub_args) = extract_subcommand(ctx.args);
        let desc = format!("git {sub}");

        if sub.is_empty() {
            return Classification::Allow(AllowReason::handler("git (no subcommand)"));
        }

        if SAFE_SUBCOMMANDS.contains(&sub.as_str()) {
            return classify_safe_subcommand(&sub, &sub_args, &desc);
        }

        if ASK_SUBCOMMANDS.contains(&sub.as_str()) {
            return Classification::Ask(desc);
        }

        // Complex subcommands with sub-subcommand analysis
        match sub.as_str() {
            "branch" => git_subcommands::classify_branch(&sub_args),
            "tag" => git_subcommands::classify_tag(&sub_args),
            "remote" => git_subcommands::classify_remote(&sub_args),
            "stash" => git_subcommands::classify_stash(&sub_args),
            "config" => git_subcommands::classify_config(&sub_args),
            "notes" => git_subcommands::classify_notes(&sub_args),
            "bisect" => git_subcommands::classify_bisect(&sub_args),
            "lfs" => git_subcommands::classify_lfs(&sub_args),
            _ => Classification::Ask(desc),
        }
    }
}

/// Dispatch a `SAFE_SUBCOMMANDS` member to its flag-aware classifier, falling
/// through to a plain `Allow` for members with no dangerous flags.
fn classify_safe_subcommand(sub: &str, args: &[String], desc: &str) -> Classification {
    GUARDED_SAFE_SUBCOMMANDS
        .iter()
        .find(|(name, _, _)| *name == sub)
        .map_or_else(
            || Classification::Allow(AllowReason::handler(desc)),
            |(_, _, classify)| classify(args, desc),
        )
}

fn classify_diff(args: &[String], desc: &str) -> Classification {
    if has_flag(args, &["--ext-diff"]) {
        return Classification::Ask("git diff --ext-diff (enables external diff driver)".into());
    }
    classify_output_path(args, &["--output"], &["-o", "--output"], desc)
}

fn classify_archive(args: &[String], desc: &str) -> Classification {
    classify_output_path(args, &["--output"], &["-o", "--output"], desc)
}

fn classify_format_patch(args: &[String], desc: &str) -> Classification {
    classify_output_path(
        args,
        &["--output-directory"],
        &["-o", "--output-directory"],
        desc,
    )
}

/// Flags whose attached (`--flag=PATH`) or separated (`--flag PATH` / `-o PATH`)
/// value names a write target; routes through `WithRedirects` so the existing
/// write-scope pipeline decides Allow (e.g. /tmp) vs Ask.
fn classify_output_path(
    args: &[String],
    eq_flags: &[&str],
    space_flags: &[&str],
    desc: &str,
) -> Classification {
    if let Some(path) = flag_path_value(args, eq_flags, space_flags) {
        return Classification::WithRedirects(AllowReason::handler(desc), vec![path]);
    }
    Classification::Allow(AllowReason::handler(desc))
}

fn flag_path_value(args: &[String], eq_flags: &[&str], space_flags: &[&str]) -> Option<String> {
    for arg in args {
        for flag in eq_flags {
            if let Some(value) = arg
                .strip_prefix(flag)
                .and_then(|rest| rest.strip_prefix('='))
            {
                return Some(value.to_owned());
            }
        }
    }
    let mut i = 0;
    while i < args.len() {
        if space_flags.contains(&args[i].as_str()) {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

/// True if `arg` is a single-dash short-flag cluster (e.g. `-nOid`, `-xid`) that
/// contains `letter` anywhere after the leading dash. Git's getopt-style short
/// options allow a value-taking flag to appear anywhere in the cluster with the
/// remaining characters as its attached value (e.g. `-nOid` = `-n -Oid`), not
/// just as the first character, so this checks containment rather than prefix.
fn short_cluster_contains(arg: &str, letter: char) -> bool {
    arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains(letter)
}

fn classify_grep(args: &[String], desc: &str) -> Classification {
    let pager_flag = args.iter().any(|a| {
        short_cluster_contains(a, 'O')
            || a == "--open-files-in-pager"
            || a.starts_with("--open-files-in-pager=")
    });
    if pager_flag {
        Classification::Ask(
            "git grep --open-files-in-pager (launches external pager/command)".into(),
        )
    } else {
        Classification::Allow(AllowReason::handler(desc))
    }
}

fn classify_difftool(args: &[String], desc: &str) -> Classification {
    let extcmd_flag = args
        .iter()
        .any(|a| short_cluster_contains(a, 'x') || a == "--extcmd" || a.starts_with("--extcmd="));
    if extcmd_flag {
        Classification::Ask("git difftool --extcmd (launches external command)".into())
    } else {
        Classification::Allow(AllowReason::handler(desc))
    }
}

/// True if `remote` is scp-like remote syntax (`user@host:path` or `host:path`),
/// which git treats as an SSH transport URL causing network egress the same as
/// an explicit `ssh://` URL. Distinguished from local refspecs (e.g.
/// `origin master:master`) by requiring no `/` before the colon, since refspecs
/// name refs (`refs/heads/...`) or branches, not bare hostnames.
fn is_scp_like_remote(remote: &str) -> bool {
    let Some(colon_idx) = remote.find(':') else {
        return false;
    };
    let host_part = &remote[..colon_idx];
    !host_part.is_empty() && !host_part.contains('/') && !host_part.contains('\\')
}

fn classify_fetch(args: &[String], desc: &str) -> Classification {
    let positionals = positional_args(args);
    if let Some(url) = positionals
        .iter()
        .find(|a| a.contains("://") || a.contains("::"))
    {
        return Classification::Ask(format!("git fetch (remote URL: {url})"));
    }
    if let Some(remote) = positionals.first().filter(|r| is_scp_like_remote(r)) {
        return Classification::Ask(format!("git fetch (remote URL: {remote})"));
    }
    Classification::Allow(AllowReason::handler(desc))
}

fn extract_subcommand(args: &[String]) -> (String, Vec<String>) {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if GLOBAL_VALUE_FLAGS.contains(&arg.as_str()) {
            i += 2; // skip flag and its value
            continue;
        }
        if GLOBAL_FLAGS.contains(&arg.as_str()) {
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        return (arg.clone(), args[i + 1..].to_vec());
    }
    (String::new(), Vec::new())
}

type SubClassifier = fn(&[String], &str) -> Classification;

/// The `SAFE_SUBCOMMANDS` members whose approval is conditional: the condition,
/// and the classifier that enforces it.
///
/// One table drives both `classify_safe_subcommand` and the catalog guard text,
/// so a new conditional subcommand cannot be documented as unconditional.
const GUARDED_SAFE_SUBCOMMANDS: &[(&str, &str, SubClassifier)] = &[
    (
        "diff",
        "no --ext-diff; an --output target runs the redirect pipeline",
        classify_diff,
    ),
    (
        "archive",
        "an -o/--output target runs the redirect pipeline",
        classify_archive,
    ),
    (
        "format-patch",
        "an -o/--output-directory target runs the redirect pipeline",
        classify_format_patch,
    ),
    ("grep", "no -O/--open-files-in-pager", classify_grep),
    ("difftool", "no -x/--extcmd", classify_difftool),
    (
        "fetch",
        "no URL-like or scp-like remote operand",
        classify_fetch,
    ),
];

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
