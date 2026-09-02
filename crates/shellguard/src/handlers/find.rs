//! Ported from rippy (MIT) https://github.com/mpecan/rippy
//!
//! Sandbox delta: rippy always Asked on `-delete` and on `-f*`-style output
//! files. Here both are allowed when every path find would touch — the
//! positional ROOT operands for `-delete`, every `-fprint`/`-fprint0`/
//! `-fprintf`/`-fls` target — resolves inside the release set
//! (`/dev/null`, `/tmp`) with the symlink re-check. `-ok`/`-okdir` stay Ask and
//! `-exec` stays Recurse, exactly as in rippy. No action short-circuits
//! another: all verdicts fold into one (#F1/#F2).

use super::{Classification, Handler, HandlerContext, has_flag, operand_in_release};
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
        // -ok/-okdir prompt per matched file; never auto-approved.
        if has_flag(ctx.args, &["-ok", "-okdir"]) {
            return Classification::Ask("find -ok (interactive)".into());
        }

        // SECURITY: no action short-circuits the others. `find /tmp -delete
        // -exec rm -rf / ;` used to Allow on the strength of the released
        // `-delete` roots while the trailing `-exec` smuggled an arbitrary
        // command past every check. Every action is folded into one verdict;
        // only an all-clean evaluation Allows. Each action leads the fold so
        // a tie keeps the action's own reason, not the bare search Allow.
        let mut class = Classification::Allow(AllowReason::handler("find (search only)"));

        if has_flag(ctx.args, &["-delete"]) {
            class = most_restrictive(delete_classification(ctx), class);
        }

        if let Some(file_class) = file_output_classification(ctx) {
            class = most_restrictive(file_class, class);
        }

        // -exec / -execdir: recurse into each inner command, keeping the
        // outer actions' verdict as the floor so a released `-delete` cannot
        // launder anything through an innocuous-looking exec.
        for (i, arg) in ctx.args.iter().enumerate() {
            if arg == "-exec" || arg == "-execdir" {
                let inner_args: Vec<&str> = ctx.args[i + 1..]
                    .iter()
                    .take_while(|a| a.as_str() != ";" && a.as_str() != "+")
                    .map(String::as_str)
                    .collect();
                if inner_args.is_empty() {
                    // Unparseable exec body: fail closed.
                    return Classification::Ask(format!("find {arg}"));
                }
                class = Classification::RecurseAtLeast(inner_args.join(" "), Box::new(class));
            }
        }

        class
    }
}

/// The more protective of two action verdicts: a released `-delete` must not
/// mask an Ask-grade `-f*` target and vice versa.
fn most_restrictive(a: Classification, b: Classification) -> Classification {
    match (&a, &b) {
        (Classification::Allow(_), Classification::Ask(_) | Classification::Deny(_))
        | (Classification::Ask(_), Classification::Deny(_)) => b,
        _ => a,
    }
}

/// The `-delete` verdict: only when every ROOT path find would delete
/// resolves inside the release set may it pass.
fn delete_classification(ctx: &HandlerContext) -> Classification {
    if roots_within_release(ctx) {
        Classification::Allow(AllowReason::ReleasedWrite(
            "find -delete within released dir".into(),
        ))
    } else {
        Classification::Ask("find -delete".into())
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

/// The targets of EVERY `-fprint`/`-fprint0`/`-fprintf`/`-fls` occurrence.
///
/// SECURITY: find accepts repeated output flags, and inspecting only the
/// first let `find / -fls /tmp/ok -fprint /etc/passwd` write the second file
/// past the release check (#F2). Every target must resolve into the release
/// set.
fn file_output_targets(ctx: &HandlerContext) -> Vec<String> {
    let mut targets = Vec::new();
    for (i, arg) in ctx.args.iter().enumerate() {
        if FILE_OUTPUT_FLAGS.contains(&arg.as_str()) {
            if let Some(target) = ctx.args.get(i + 1) {
                targets.push(target.clone());
            }
        }
    }
    targets
}

fn file_output_classification(ctx: &HandlerContext) -> Option<Classification> {
    let targets = file_output_targets(ctx);
    if targets.is_empty() {
        return None;
    }
    if targets
        .iter()
        .all(|t| operand_in_release(t, ctx.working_directory, ctx.safe_scopes))
    {
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
        assert!(matches!(result, Classification::RecurseAtLeast(cmd, _) if cmd == "wc -l {}"));
    }

    /// #F1: `-delete` may not short-circuit later actions. The released roots
    /// approved the whole command before the trailing `-exec` was ever seen.
    #[test]
    fn delete_may_not_short_circuit_a_later_exec() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at(
                "/project",
                &["/tmp", "-delete", "-exec", "rm", "-rf", "/", ";"]
            )),
            Classification::RecurseAtLeast(cmd, _) if cmd == "rm -rf /"
        ));
    }

    /// #F1: `-delete` may not short-circuit later `-f*` output targets.
    #[test]
    fn delete_may_not_short_circuit_a_later_file_output() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/tmp", "-delete", "-fls", "/etc/x"])),
            Classification::Ask(desc) if desc == "find (writes to file)"
        ));
    }

    /// #F1: the pure released form stays allowed (regression floor).
    #[test]
    fn delete_alone_inside_the_release_set_still_allows() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/tmp", "-delete"])),
            Classification::Allow(r) if r.to_string().contains("find -delete within released dir")
        ));
    }

    /// #F1: an unreleased root must keep its Ask even when a harmless exec
    /// would otherwise Allow the whole command.
    #[test]
    fn unreleased_delete_root_survives_a_harmless_exec() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at(
                "/project",
                &["/var", "-delete", "-exec", "echo", "{}", ";"]
            )),
            Classification::RecurseAtLeast(_, outer)
                if matches!(&*outer, Classification::Ask(desc) if desc == "find -delete")
        ));
    }

    /// #F1: every `-exec` occurrence recurses, not just the first.
    #[test]
    fn every_exec_occurrence_recurses() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at(
                "/project",
                &["/tmp", "-exec", "echo", "{}", ";", "-exec", "wc", "-l", "{}", ";"]
            )),
            Classification::RecurseAtLeast(cmd, _) if cmd == "wc -l {}"
        ));
    }

    /// #F1: an `-exec` body that never terminates fails closed.
    #[test]
    fn unterminated_exec_body_asks() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at("/project", &["/tmp", "-exec"])),
            Classification::Ask(desc) if desc == "find -exec"
        ));
    }

    /// #F2: every `-f*` target is checked, so a released first target cannot
    /// launder a second write outside the release set.
    #[test]
    fn a_released_first_file_output_cannot_launder_a_second() {
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at(
                "/project",
                &["/", "-fls", "/tmp/ok", "-fprint", "/etc/passwd"]
            )),
            Classification::Ask(desc) if desc == "find (writes to file)"
        ));
        // Both targets released: still allowed.
        assert!(matches!(
            FIND_HANDLER.classify(&ctx_at(
                "/project",
                &["/", "-fls", "/tmp/ok", "-fprint", "/tmp/also-ok"]
            )),
            Classification::Allow(r) if r.to_string().contains("find output file within released dir")
        ));
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
