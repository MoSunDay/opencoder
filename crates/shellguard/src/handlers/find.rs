//! Ported from rippy (MIT) https://github.com/mpecan/rippy
//!
//! Sandbox delta: rippy always Asked on `-delete` and on `-f*`-style output
//! files. Here both are allowed when every path find would touch — the
//! positional ROOT operands for `-delete`, the file target for
//! `-fprint`/`-fprint0`/`-fprintf`/`-fls` — resolves inside the release set
//! (`/dev/null`, `/tmp`) with the symlink re-check. `-ok`/`-okdir` stay Ask and
//! `-exec` stays Recurse, exactly as in rippy.

use super::{has_flag, operand_in_release, Classification, Handler, HandlerContext};
use crate::verdict::AllowReason;

pub(crate) static FIND_HANDLER: FindHandler = FindHandler;

pub(crate) struct FindHandler;

/// Flags whose *value* is a file find writes its results into.
const FILE_OUTPUT_FLAGS: &[&str] = &["-fprint", "-fprint0", "-fprintf", "-fls"];

impl Handler for FindHandler {
    fn commands(&self) -> &[&str] {
        &["find"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_flag(ctx.args, &["-delete"]) {
            // Only the path arguments (roots) get deleted; the release check
            // covers each of them, so `find /tmp -delete` may pass.
            if roots_within_release(ctx) {
                return Classification::Allow(AllowReason::ReleasedWrite(
                    "find -delete within released dir".into(),
                ));
            }
            return Classification::Ask("find -delete".into());
        }

        if has_flag(ctx.args, &["-ok", "-okdir"]) {
            return Classification::Ask("find -ok (interactive)".into());
        }

        if let Some(class) = file_output_classification(ctx) {
            return class;
        }

        // -exec / -execdir: extract inner command and delegate
        for (i, arg) in ctx.args.iter().enumerate() {
            if arg == "-exec" || arg == "-execdir" {
                let inner_args: Vec<&str> = ctx.args[i + 1..]
                    .iter()
                    .take_while(|a| a.as_str() != ";" && a.as_str() != "+")
                    .map(String::as_str)
                    .collect();
                if !inner_args.is_empty() {
                    return Classification::Recurse(inner_args.join(" "));
                }
                return Classification::Ask(format!("find {arg}"));
            }
        }

        Classification::Allow(AllowReason::handler("find (search only)"))
    }
}

/// find's path arguments: the leading non-flag operands, before the first
/// option starts the expression. `find [-H] [-L] path... [expression]`.
fn roots(ctx: &HandlerContext) -> Vec<String> {
    let mut paths = Vec::new();
    for arg in ctx.args {
        if arg.starts_with('-') {
            break;
        }
        paths.push(arg.clone());
    }
    paths
}

fn roots_within_release(ctx: &HandlerContext) -> bool {
    let paths = roots(ctx);
    !paths.is_empty()
        && paths
            .iter()
            .all(|p| operand_in_release(p, ctx.working_directory, ctx.safe_scopes))
}

/// The file target of `-fprint`/`-fprint0`/`-fprintf`/`-fls`, if present.
fn file_output_target(ctx: &HandlerContext) -> Option<String> {
    for (i, arg) in ctx.args.iter().enumerate() {
        if FILE_OUTPUT_FLAGS.contains(&arg.as_str()) {
            return ctx.args.get(i + 1).cloned();
        }
    }
    None
}

fn file_output_classification(ctx: &HandlerContext) -> Option<Classification> {
    let target = file_output_target(ctx)?;
    if operand_in_release(&target, ctx.working_directory, ctx.safe_scopes) {
        return Some(Classification::Allow(AllowReason::ReleasedWrite(
            "find output file within released dir".into(),
        )));
    }
    Some(Classification::Ask("find (writes to file)".into()))
}

#[cfg(test)]
mod tests {

    use super::*;

    fn ctx_at<'a>(cwd: &'a str, argv: &[&str]) -> HandlerContext<'a> {
        let owned: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        HandlerContext {
            command_name: "find",
            args: owned.leak(),
            working_directory: std::path::Path::new(cwd),
            remote: false,
            safe_scopes: &[],
        }
    }

    // find search/-delete/-ok command->decision cases are covered by
    // rippy's command catalog (not ported). These tests assert the Recurse
    // variant and exact inner-command extraction, which a command string can't check.
    #[test]
    fn find_exec_recurses() {
        let args: Vec<String> = vec![
            ".".into(),
            "-name".into(),
            "*.rs".into(),
            "-exec".into(),
            "wc".into(),
            "-l".into(),
            "{}".into(),
            ";".into(),
        ];
        let result = FIND_HANDLER.classify(&HandlerContext::test("find", &args));
        assert!(matches!(result, Classification::Recurse(cmd) if cmd == "wc -l {}"));
    }

    #[test]
    fn delete_inside_the_release_set_allows_outside_asks() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/tmp", "-delete"])),
            Classification::Allow(r) if r.to_string().contains("find -delete within released dir")
        ));
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/", "-delete"])),
            Classification::Ask(desc) if desc == "find -delete"
        ));
        // A cwd root is never released in sandbox mode.
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &[".", "-delete"])),
            Classification::Ask(_)
        ));
        // Every root must pass: one outside and the whole command asks.
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/tmp", "/var", "-delete"])),
            Classification::Ask(_)
        ));
    }

    #[test]
    fn file_output_targets_follow_the_release_set() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &[".", "-name", "x", "-fls", "/tmp/f.out"])),
            Classification::Allow(r) if r.to_string().contains("find output file within released dir")
        ));
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &[".", "-fprint", "out.txt"])),
            Classification::Ask(desc) if desc == "find (writes to file)"
        ));
    }

    #[test]
    fn interactive_exec_flags_stay_gated() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/tmp", "-ok", "rm", "{}", ";"])),
            Classification::Ask(_)
        ));
    }
}
