//! Sandbox release handler: destructive/file-creating commands whose operands
//! all land inside the released directories (`/dev/null`, `/tmp`).
//!
//! New in the sandbox port: rippy gated `rm`/`mv`/`cp`/`touch`/`chmod`/...
//! through user-supplied ASK-only stdlib TOML rules (not ported — the sandbox
//! has no config layer). Without a handler those commands would fall to the
//! fail-closed default Ask even when every operand is already released, so
//! this handler allows exactly the released subset and Asks otherwise.

use super::{Classification, Handler, HandlerContext, operand_in_release, positional_operands};
use crate::verdict::AllowReason;

pub(crate) static SANDBOX_RELEASE_HANDLER: SandboxReleaseHandler = SandboxReleaseHandler;

pub(crate) struct SandboxReleaseHandler;

/// Commands that create, destroy or re-permission files: with every operand
/// inside the release set they are pure sandbox noise, anywhere else they
/// must be confirmed.
const COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "touch", "ln", "chmod", "chown", "chgrp", "install", "truncate",
];

/// Flags in this command set whose following token is metadata (a mode, an
/// owner, a size, a timestamp) rather than a path. `-t` is deliberately NOT
/// listed: for `mv`/`cp`/`install` it names a target *directory*, which must
/// be checked like any other write target.
fn metadata_value_flags(command: &str) -> &'static [&'static str] {
    match command {
        // install: mode / owner / group / suffix. truncate: size.
        "install" => &["-m", "-o", "-g", "-S"],
        "truncate" => &["-s"],
        // touch: date and timestamp values.
        "touch" => &["-d", "-t"],
        // cp/mv: rename-suffix value.
        "cp" | "mv" => &["-S"],
        _ => &[],
    }
}

/// The path operands of a release command: every positional token, minus
/// metadata flag values, minus a leading MODE/OWNER spec that is not a path
/// (`chmod +x f`, `chown user:group f`, `chgrp staff f`).
fn path_operands(ctx: &HandlerContext) -> Vec<String> {
    let mut operands =
        positional_operands(ctx.args, metadata_value_flags(ctx.command_name));
    if matches!(ctx.command_name, "chmod" | "chown" | "chgrp") {
        if let Some(first) = operands.first() {
            let spec = match ctx.command_name {
                "chmod" => is_chmod_mode(first),
                _ => is_owner_spec(first),
            };
            if spec {
                operands.remove(0);
            }
        }
    }
    operands
}

/// A `chmod` MODE word: octal (`755`, `0644`) or a comma list of symbolic
/// clauses (`+x`, `a+r`, `u+x,go-w`). Anything path-shaped contains `/`.
fn is_chmod_mode(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    token.split(',').all(|clause| {
        let body = clause.trim_start_matches(['u', 'g', 'o', 'a']);
        let mut chars = body.chars();
        match chars.next() {
            Some('+' | '-' | '=') => {}
            _ => return false,
        }
        chars.all(|c| matches!(c, 'r' | 'w' | 'x' | 'X' | 's' | 't' | 'u' | 'g' | 'o'))
    })
}

/// A `chown`/`chgrp` owner/group spec: `user`, `user:group`, `:group`,
/// `.group`, or numeric ids. A leading path operand always contains `/`.
fn is_owner_spec(token: &str) -> bool {
    if token.contains('/') || token.is_empty() {
        return false;
    }
    let spec = token.strip_prefix('.').unwrap_or(token);
    spec.split(':').all(|part| {
        !part.is_empty()
            && part.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'@' | b'+' | b'#')
            })
    })
}

impl Handler for SandboxReleaseHandler {
    fn commands(&self) -> &[&str] {
        COMMANDS
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        let operands = path_operands(ctx);
        if operands.is_empty() {
            // `rm` with nothing to act on is a usage error; confirm it.
            return Classification::Ask(format!("{} without file operands", ctx.command_name));
        }
        let mut offending: Option<&String> = None;
        for operand in &operands {
            if !operand_in_release(operand, ctx.working_directory, ctx.safe_scopes)
                && offending.is_none()
            {
                offending = Some(operand);
            }
        }
        match offending {
            None => Classification::Allow(AllowReason::ReleasedWrite(format!(
                "{} within released dir",
                ctx.command_name
            ))),
            Some(path) => Classification::Ask(format!(
                "{} outside released dir ({path})",
                ctx.command_name
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::{is_within_safe_dir, normalize_path};

    fn classify_at(cwd: &str, argv: &[&str]) -> Classification {
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
        let rest = args.get(1..).unwrap_or(&[]);
        let command: &str = argv.first().copied().unwrap_or("rm");
        let ctx = HandlerContext {
            command_name: command,
            args: rest,
            working_directory: std::path::Path::new(cwd),
            remote: false,
            safe_scopes: &[],
        };
        SANDBOX_RELEASE_HANDLER.classify(&ctx)
    }

    #[test]
    fn released_operands_allow_and_others_ask() {
        assert!(matches!(
            classify_at("/project", &["rm", "-rf", "/tmp/d"]),
            Classification::Allow(_)
        ));
        assert!(matches!(
            classify_at("/project", &["mv", "/tmp/a", "/tmp/b"]),
            Classification::Allow(_)
        ));
        // Sources count, not just destinations.
        assert!(matches!(
            classify_at("/project", &["mv", "/etc/x", "/tmp/y"]),
            Classification::Ask(desc) if desc == "mv outside released dir (/etc/x)"
        ));
        assert!(matches!(
            classify_at("/project", &["chmod", "+x", "/tmp/a.sh"]),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn relative_operands_resolve_against_the_working_directory() {
        assert!(matches!(
            classify_at("/project", &["rm", "x"]),
            Classification::Ask(desc) if desc == "rm outside released dir (x)"
        ));
        assert!(matches!(
            classify_at("/tmp", &["rm", "x"]),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn flag_values_and_the_separator_are_handled() {
        // `--` ends flag parsing: everything after is an operand.
        assert!(matches!(
            classify_at("/project", &["rm", "--", "/tmp/x"]),
            Classification::Allow(_)
        ));
        assert!(matches!(
            classify_at("/project", &["rm", "--", "-rf"]),
            Classification::Ask(_)
        ));
        // Glued/valued flags are skipped, never treated as operands.
        assert!(matches!(
            classify_at("/project", &["install", "-m644", "/tmp/a", "/tmp/b"]),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn component_boundary_prefixes_stay_outside_the_release_set() {
        assert!(matches!(
            classify_at("/project", &["touch", "/tmpx"]),
            Classification::Ask(_)
        ));
        assert!(matches!(
            classify_at("/project", &["truncate", "-s0", "/dev/null"]),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn normalize_path_is_reused_for_logical_parents() {
        let args: Vec<String> = vec!["rm".into(), "/tmp/../etc/x".into()];
        let ctx = HandlerContext {
            command_name: "rm",
            args: &args[1..],
            working_directory: std::path::Path::new("/project"),
            remote: false,
            safe_scopes: &[],
        };
        let operands = path_operands(&ctx);
        assert_eq!(operands, vec!["/tmp/../etc/x".to_owned()]);
        let resolved = normalize_path(std::path::Path::new("/tmp/../etc/x"));
        assert!(!is_within_safe_dir(&resolved, ctx.safe_scopes));
        assert!(matches!(
            SANDBOX_RELEASE_HANDLER.classify(&ctx),
            Classification::Ask(_)
        ));
    }
}
