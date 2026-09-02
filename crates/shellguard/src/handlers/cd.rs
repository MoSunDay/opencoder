//! `cd`/`pushd` destination scoping. `cd` itself writes nothing: it only
//! re-aims the shell cwd for this invocation, and the analyzer re-aims the
//! analysis cwd the same way, so a `cd` whose destination is statically
//! resolvable is read-only and passes both sandbox and plan policy.
//! Unresolvable destinations (variables, `~`, no-args home, unknown flags)
//! still ask — later relative operands would be unjudgeable. `pushd` keeps
//! destination scoping (safe scopes + release dirs); `popd` asks.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use std::path::Path;

use super::{is_within_safe_dir, normalize_path, Classification, Handler, HandlerContext};
use crate::verdict::AllowReason;

pub(crate) static CD_HANDLER: CdHandler = CdHandler;

pub(crate) struct CdHandler;

/// `cd` option flags that take no value and don't change the destination.
const CD_KNOWN_FLAGS: &[&str] = &["-L", "-P", "-e", "-@"];

/// Skip leading `cd` option tokens (and a `--` terminator) to find the real
/// destination. Returns `None` (fail closed) if a leading flag is not one of
/// the known no-op flags, since an unrecognized flag could shift or consume
/// the destination in ways this handler can't reason about.
///
/// Shared with the analyzer's cwd re-aim (`extract_cd_target`), so both agree
/// on where a `cd` lands — `cd -P src && touch f` must re-aim at `src`, not
/// at a literal `-P` directory (#F9).
pub(crate) fn resolve_target(args: &[String]) -> Option<&String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            return args.get(i + 1);
        }
        if arg == "-" || !arg.starts_with('-') {
            return Some(arg);
        }
        if !CD_KNOWN_FLAGS.contains(&arg.as_str()) {
            return None;
        }
        i += 1;
    }
    None
}

impl Handler for CdHandler {
    fn commands(&self) -> &[&str] {
        &["cd", "pushd", "popd"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if ctx.command_name == "popd" {
            return Classification::Ask("popd (unknown destination)".into());
        }

        if ctx.remote {
            return Classification::Ask(format!("{} in remote context", ctx.command_name));
        }

        if ctx.args.is_empty() {
            return Classification::Ask(format!("{} (goes to home directory)", ctx.command_name));
        }

        let Some(target) = resolve_target(ctx.args) else {
            return Classification::Ask(format!("{} (unknown flag)", ctx.command_name));
        };

        if target == "-" {
            return Classification::Allow(AllowReason::handler(format!(
                "{} - (previous directory)",
                ctx.command_name
            )));
        }

        // Can't statically resolve the destination
        if target.contains('$') || target.contains('`') {
            return Classification::Ask(format!("{} with variable expansion", ctx.command_name));
        }

        if target.starts_with('~') {
            return Classification::Ask(format!("{} to home directory", ctx.command_name));
        }

        let resolved = if Path::new(target).is_absolute() {
            normalize_path(Path::new(target))
        } else {
            normalize_path(&ctx.working_directory.join(target))
        };

        // `cd` with a statically resolvable destination is read-only: no
        // filesystem state changes, and `analyze_list` re-aims the analysis
        // cwd from this same target, so every later relative operand is
        // still judged against the directory it would really land in.
        if ctx.command_name == "cd" {
            return Classification::Allow(AllowReason::handler(format!("cd to {target}")));
        }

        // Sandbox policy (`pushd`): the working directory is NOT a writable
        // scope, so only a declared safe scope (or a built-in release dir)
        // auto-passes; the dirstack effect is not reasoned about.
        if is_within_safe_dir(&resolved, ctx.safe_scopes) {
            Classification::Allow(AllowReason::handler(format!(
                "{} within allowed scope",
                ctx.command_name
            )))
        } else {
            Classification::Ask(format!("{} to {target}", ctx.command_name))
        }
    }
}

#[cfg(test)]
#[path = "cd_tests.rs"]
mod tests;
