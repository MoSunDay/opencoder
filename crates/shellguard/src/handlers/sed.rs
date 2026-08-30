//! `sed` expression screening: in-place edits ask; `w`/`W` write-backs, `e`
//! command execution and dangerous flags are rejected.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    Classification, Handler, HandlerContext, has_flag, has_flag_or_prefixed, has_glued_short_flag,
};
use crate::verdict::AllowReason;
pub(crate) static SED_HANDLER: SedHandler = SedHandler;

pub(crate) struct SedHandler;

impl Handler for SedHandler {
    fn commands(&self) -> &[&str] {
        &["sed"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_in_place_edit(ctx.args) {
            return Classification::Ask("sed -i (in-place edit)".into());
        }

        // Scan sed expressions for dangerous commands
        if let Some(reason) = check_sed_expression(ctx.args) {
            return Classification::Ask(reason);
        }

        Classification::Allow(AllowReason::handler("sed (filter)"))
    }

}

/// In-place edit detection: the bare short flag (`-i`), a glued backup suffix
/// (`-i.bak`, `-i''`-style forms), and the GNU long forms `--in-place` /
/// `--in-place=<suffix>`.
///
/// SECURITY: sed has no other flag beginning with `-i`, so any single-dash
/// token prefixed `-i` (length 2 exactly, or longer with the glued suffix) IS
/// the in-place flag — `has_flag` covers the exact match and
/// `has_glued_short_flag` the glued-suffix form. The long form is matched via
/// `has_flag_or_prefixed` so both `--in-place` and `--in-place=.bak` hit
/// without swallowing an unrelated longer option.
fn is_in_place_edit(args: &[String]) -> bool {
    has_flag(args, &["-i"])
        || has_glued_short_flag(args, &["-i"])
        || has_flag_or_prefixed(args, &["--in-place"])
}

/// Check sed expression arguments for `w`/`e` flags on `s///` and bare `e`/`w`
/// (possibly address-prefixed) commands.
fn check_sed_expression(args: &[String]) -> Option<String> {
    for arg in args {
        if arg.starts_with('-') {
            continue;
        }
        for cmd in arg.split(['\n', ';']) {
            if let Some(reason) = check_sed_command(cmd.trim()) {
                return Some(reason);
            }
        }
    }
    None
}

/// Check a single (already semicolon-split) sed command segment.
fn check_sed_command(cmd: &str) -> Option<String> {
    if sed_has_dangerous_flag(cmd) {
        return Some("sed w/e flag (writes to file or executes)".into());
    }
    let rest = strip_sed_address(cmd);
    if is_bare_e_command(rest) {
        return Some("sed e (shell execution)".into());
    }
    if rest == "w" || rest.starts_with("w ") {
        return Some("sed w (writes to file)".into());
    }
    None
}

/// Strip a leading sed address (`N`, `N,M`, `$`, `/regex/`, optionally
/// followed by `!`) from a command segment, returning the remaining command.
fn strip_sed_address(cmd: &str) -> &str {
    let bytes = cmd.as_bytes();
    let mut i = 0;
    let len = bytes.len();

    if i < len && bytes[i] == b'/' {
        i += 1;
        while i < len && bytes[i] != b'/' {
            i += 1;
        }
        if i < len {
            i += 1; // skip closing '/'
        }
    } else {
        while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b',' || bytes[i] == b'$') {
            i += 1;
        }
    }
    if i < len && bytes[i] == b'!' {
        i += 1;
    }
    cmd[i..].trim_start()
}

/// Check whether a (address-stripped) command is a bare `e` execute command,
/// e.g. `e`, `e cmd`.
fn is_bare_e_command(rest: &str) -> bool {
    rest == "e" || rest.starts_with("e ") || rest.starts_with("e\t")
}

/// Check if a sed `s` command has a `w` (write) or `e` (execute) flag after
/// the third delimiter. e.g., `s/foo/bar/gw output.txt` or `s/x/id/e` — the
/// flag is in the flags section, after the replacement text ends.
/// Avoids false positives like `s/foo/w bar/` where `w` is in the replacement.
fn sed_has_dangerous_flag(expr: &str) -> bool {
    let cmd = expr.trim();
    let mut chars = cmd.char_indices();
    if chars.next().map(|(_, c)| c) != Some('s') {
        return false;
    }
    // A byte delimiter would match the lead byte of a multi-byte scalar and
    // then slice mid-character; see docs/security-invariants.md#non-ascii-inline-code.
    let Some((_, delim)) = chars.next() else {
        return false;
    };
    let mut count = 0u8;
    for (i, c) in chars {
        if c == delim {
            count += 1;
            if count == 2 {
                let flags = &cmd[i + delim.len_utf8()..];
                return flags.contains('w') || flags.contains('e');
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // Inline sed/awk command->decision cases are covered by rippy's catalog
    // (not ported). The tests below exercise internals directly.

    /// Every GNU/BSD in-place spelling must Ask regardless of the operand
    /// path, so the context points at a non-release working directory
    /// (`HandlerContext::test` defaults to `/tmp`, which the sandbox releases).
    fn non_release_ctx(args: &[String]) -> HandlerContext<'_> {
        HandlerContext {
            working_directory: std::path::Path::new("/home/u/project"),
            ..HandlerContext::test("sed", args)
        }
    }

    fn asks(args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(SED_HANDLER.classify(&non_release_ctx(&args)), Classification::Ask(_)),
            "expected Ask for {args:?}"
        );
    }

    fn allows(args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(SED_HANDLER.classify(&non_release_ctx(&args)), Classification::Allow(_)),
            "expected Allow for {args:?}"
        );
    }

    #[test]
    fn every_in_place_spelling_asks() {
        asks(&["-i", "s/a/b/", "f"]);
        asks(&["-i.bak", "s/a/b/", "f"]); // glued suffix value
        asks(&["-i", ".bak", "s/a/b/", "f"]); // separate suffix token
        asks(&["--in-place", "s/a/b/", "f"]);
        asks(&["--in-place=.bak", "s/a/b/", "f"]);
        // The in-place check wins over an otherwise-safe expression.
        asks(&["-e", "s/a/b/", "-i", "f"]);
    }

    #[test]
    fn read_only_sed_still_allows() {
        allows(&["s/a/b/", "f"]); // no -i
        allows(&["-n", "1p", "f"]);
        allows(&["-e", "s/a/b/", "f"]); // --expression style still screened
        // Tokens that merely begin with a dash but are not `-i` stay allowed.
        allows(&["--quiet", "s/a/b/", "f"]);
    }

    /// A multi-byte delimiter used to panic here: the delimiter was read as a
    /// single byte, so it matched the lead byte of each `ї` and produced a
    /// flags offset inside a scalar. See docs/security-invariants.md#non-ascii-inline-code.
    #[test]
    fn non_ascii_sed_delimiter_is_classified_without_panicking() {
        assert!(!sed_has_dangerous_flag("sїaїbїc"));
        assert!(sed_has_dangerous_flag("sїaїbїw"));
        assert!(!sed_has_dangerous_flag("s/foo/w bar/"));
        assert!(sed_has_dangerous_flag("s/foo/bar/gw out.txt"));
    }
}
