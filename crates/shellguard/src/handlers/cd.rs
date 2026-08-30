//! `cd`/`pushd` destination scoping: allow only `cd -`, declared safe scopes,
//! and the built-in release directories; everything else asks.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use std::path::Path;

use super::{Classification, Handler, HandlerContext, is_within_safe_dir, normalize_path};
use crate::verdict::AllowReason;

pub(crate) static CD_HANDLER: CdHandler = CdHandler;

pub(crate) struct CdHandler;

/// `cd` option flags that take no value and don't change the destination.
const CD_KNOWN_FLAGS: &[&str] = &["-L", "-P", "-e", "-@"];

/// Skip leading `cd` option tokens (and a `--` terminator) to find the real
/// destination. Returns `None` (fail closed) if a leading flag is not one of
/// the known no-op flags, since an unrecognized flag could shift or consume
/// the destination in ways this handler can't reason about.
fn resolve_target(args: &[String]) -> Option<&String> {
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

        // Sandbox policy: the working directory is NOT a writable scope, so
        // only a declared safe scope (or a built-in release dir) auto-passes.
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
