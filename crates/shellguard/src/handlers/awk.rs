//! `awk`/`gawk`/`mawk` program screening: `system()`, pipes to commands and
//! file redirects in inline programs or `-f` script files ask.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{Classification, Handler, HandlerContext, get_flag_value, has_flag_or_prefixed, has_glued_short_flag};
use crate::verdict::AllowReason;
pub(crate) static AWK_HANDLER: AwkHandler = AwkHandler;

pub(crate) struct AwkHandler;

impl Handler for AwkHandler {
    fn commands(&self) -> &[&str] {
        &["awk", "gawk", "mawk", "nawk"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_awk_flag(ctx.args, "-f", &["-f", "--file"]) {
            let program = awk_script_path(ctx.args).and_then(|path| ctx.read_file(&path));
            return program.map_or_else(
                || Classification::Ask(format!("{} -f (script file)", ctx.command_name)),
                |program| check_awk_source(&program, ctx.command_name),
            );
        }

        if has_awk_flag(ctx.args, "-l", &["-l", "--load"]) {
            return Classification::Ask(format!("{} -l (loads shared library)", ctx.command_name));
        }

        if let Some(reason) = check_awk_include(ctx.args, ctx.command_name) {
            return Classification::Ask(reason);
        }

        if let Some(reason) = check_awk_program(ctx.args, ctx.command_name) {
            return Classification::Ask(reason);
        }

        Classification::Allow(AllowReason::handler(format!(
            "{} (filter)",
            ctx.command_name
        )))
    }

}

/// Whether a code-loading flag is present in any spelling: bare (`-f`),
/// `flag=value`, or glued short (`-fscript`). A flag with no value at all still
/// counts, so a malformed invocation cannot fall through to the filter surface.
fn has_awk_flag(args: &[String], short: &str, spellings: &[&str]) -> bool {
    has_flag_or_prefixed(args, spellings) || has_glued_short_flag(args, &[short])
}

/// Extract the script path from any spelling of `-f`: `-f s`, `-fs`,
/// `--file s`, `--file=s`.
fn awk_script_path(args: &[String]) -> Option<String> {
    get_flag_value(args, &["-f", "--file"]).or_else(|| attached_flag_value(args, "-f", "--file="))
}

/// Check for gawk's `-i`/`--include`, which either rewrites the input file in
/// place (`-i inplace`) or loads an arbitrary awk source file. Unlike `-f`'s
/// plain relative path, `-i` searches `AWKPATH`, so a same-named local file is
/// not trustworthy evidence of what gawk actually loads — always Ask.
fn check_awk_include(args: &[String], cmd_name: &str) -> Option<String> {
    let value = get_flag_value(args, &["-i", "--include"])
        .or_else(|| attached_flag_value(args, "-i", "--include="))?;
    if value == "inplace" || value.starts_with("inplace:") {
        Some(format!("{cmd_name} -i inplace (rewrites file)"))
    } else {
        Some(format!("{cmd_name} -i (include file)"))
    }
}

/// Extract the value attached to a flag: glued to the short form (`-iinplace`)
/// or joined to the long form with `=` (`--include=inplace`).
fn attached_flag_value(args: &[String], short: &str, long_prefix: &str) -> Option<String> {
    for arg in args {
        if let Some(value) = arg.strip_prefix(long_prefix) {
            return Some(value.to_owned());
        }
        if let Some(value) = arg.strip_prefix(short) {
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Check an awk source string for dangerous patterns.
fn check_awk_source(program: &str, cmd_name: &str) -> Classification {
    if awk_has_system_call(program) {
        return Classification::Ask(format!("{cmd_name} -f system() (shell execution)"));
    }
    if awk_has_pipe_to_command(program) {
        return Classification::Ask(format!("{cmd_name} -f pipe to command"));
    }
    if awk_has_file_redirect(program) {
        return Classification::Ask(format!("{cmd_name} -f file redirect"));
    }
    Classification::Allow(AllowReason::handler(format!("{cmd_name} -f (safe script)")))
}

/// Check awk program arguments for `system()`, pipe-to-command, and file redirects.
fn check_awk_program(args: &[String], cmd_name: &str) -> Option<String> {
    for arg in args {
        if arg.starts_with('-') {
            continue;
        }
        if awk_has_system_call(arg) {
            return Some(format!("{cmd_name} system() (shell execution)"));
        }
        if awk_has_pipe_to_command(arg) {
            return Some(format!("{cmd_name} pipe to command"));
        }
        if awk_has_file_redirect(arg) {
            return Some(format!("{cmd_name} file redirect"));
        }
    }
    None
}

/// Detect an awk `system(...)` call, tolerating whitespace between the
/// function name and its opening paren (`system ("id")` is valid awk syntax
/// and was missed by a plain `"system("` substring match).
fn awk_has_system_call(program: &str) -> bool {
    let bytes = program.as_bytes();
    let Some(mut idx) = program.find("system") else {
        return false;
    };
    loop {
        let before_ok =
            idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric() && bytes[idx - 1] != b'_';
        let after = &program[idx + 6..];
        let trimmed = after.trim_start();
        if before_ok && trimmed.starts_with('(') {
            return true;
        }
        match program[idx + 6..].find("system") {
            Some(next) => idx = idx + 6 + next,
            None => return false,
        }
    }
}

/// A byte is a "word" character for the purposes of distinguishing awk's `/`
/// division operator (follows an identifier/number/`)`/`]`/`$`) from a `/regex/`
/// literal start (follows anything else, including the start of the program).
const fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b')' || b == b']' || b == b'$'
}

/// Skip over a double-quoted string literal starting at `quote_idx` (the
/// opening `"`), honoring backslash escapes. Returns the index just past the
/// closing quote (or the end of the program if unterminated).
fn skip_awk_string(bytes: &[u8], quote_idx: usize) -> usize {
    let mut j = quote_idx + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        if bytes[j] == b'"' {
            return j + 1;
        }
        j += 1;
    }
    j
}

/// Skip over a `/regex/` literal starting at `slash_idx` (the opening `/`),
/// honoring backslash escapes. Returns the index just past the closing `/`
/// (or the end of the program if unterminated).
fn skip_awk_regex(bytes: &[u8], slash_idx: usize) -> usize {
    let mut j = slash_idx + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        if bytes[j] == b'/' {
            return j + 1;
        }
        j += 1;
    }
    j
}

/// Detect awk pipe-to-command patterns: `print ... | "cmd"`, `"cmd" | getline`,
/// `print | cmd_var`. Awk's grammar has no bitwise/single-pipe operator other
/// than pipe-to-command / pipe-from-command-into-getline, so any `|` that is
/// not part of a `||` logical-or and not inside a string or `/regex/` literal
/// (where `|` is ordinary alternation, e.g. `/foo|bar/`) is unconditionally
/// one of those two forms.
fn awk_has_pipe_to_command(program: &str) -> bool {
    let bytes = program.as_bytes();
    let mut prev_significant: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            i = skip_awk_string(bytes, i);
            prev_significant = Some(b'"');
            continue;
        }
        if b == b'/' && !matches!(prev_significant, Some(c) if is_word_byte(c)) {
            i = skip_awk_regex(bytes, i);
            prev_significant = Some(b'/');
            continue;
        }
        if b == b'|' {
            let prev_is_pipe = i > 0 && bytes[i - 1] == b'|';
            let next_is_pipe = i + 1 < bytes.len() && bytes[i + 1] == b'|';
            if !prev_is_pipe && !next_is_pipe {
                return true;
            }
        }
        if !b.is_ascii_whitespace() {
            prev_significant = Some(b);
        }
        i += 1;
    }
    false
}

/// Detect awk file redirect patterns: `print ... > "file"` / `>> "file"`
/// (destination quoted, optionally space-delimited), plus the no-space form
/// where the redirect operator immediately follows a closing quote
/// (`"x">"/tmp/evil"`). Anchored to a quote adjacent to the `>`/`>>` (not a
/// bare operator) so numeric/string comparisons like `$1 > 100` or
/// `a >= b` keep Allowing.
fn awk_has_file_redirect(program: &str) -> bool {
    if program.contains(">> \"")
        || program.contains(">>\"")
        || program.contains(" > \"")
        || program.contains("\t> \"")
    {
        return true;
    }

    let bytes = program.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        // Only check the first '>' of a possible ">>" run.
        if b != b'>' || (i > 0 && bytes[i - 1] == b'>') {
            continue;
        }
        let mut k = i;
        while k > 0 && bytes[k - 1].is_ascii_whitespace() {
            k -= 1;
        }
        if k > 0 && bytes[k - 1] == b'"' {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::handlers::test_support::{cleanup_dir, temp_dir, write_file};

    // Inline sed/awk command->decision cases are covered by
    // rippy's command catalog (not ported). The awk `-f` tests below
    // exercise read_file on real script content, which the catalog cannot inject.

    #[test]
    fn awk_f_safe_file_allows() {
        let dir = temp_dir("awk-safe");
        write_file(&dir, "safe.awk", "{print $1}");
        let args: Vec<String> = vec!["-f".into(), "safe.awk".into(), "data.txt".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("awk", &args)
        };
        let result = AWK_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Allow(_)));
        cleanup_dir(&dir);
    }

    #[test]
    fn awk_f_system_file_asks() {
        let dir = temp_dir("awk-evil");
        write_file(&dir, "evil.awk", r#"{system("rm -rf /")}"#);
        let args: Vec<String> = vec!["-f".into(), "evil.awk".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("awk", &args)
        };
        let result = AWK_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Ask(_)));
        cleanup_dir(&dir);
    }

    #[test]
    fn awk_regex_alternation_is_not_pipe() {
        assert!(!awk_has_pipe_to_command("/foo|bar/ {print}"));
    }

    #[test]
    fn awk_regex_alternation_field_match_is_not_pipe() {
        assert!(!awk_has_pipe_to_command("$1 ~ /a|b/ {print}"));
    }

    #[test]
    fn awk_regex_alternation_in_gsub_is_not_pipe() {
        assert!(!awk_has_pipe_to_command(r#"{gsub(/x|y/,"z")}"#));
    }

    #[test]
    fn awk_string_literal_pipe_is_not_pipe_to_command() {
        assert!(!awk_has_pipe_to_command(r#"BEGIN{print "a|b"}"#));
    }

    #[test]
    fn awk_pipe_to_command_still_detected() {
        assert!(awk_has_pipe_to_command(r#"{print $0 | "sort"}"#));
    }

    #[test]
    fn awk_pipe_to_command_no_space_still_detected() {
        assert!(awk_has_pipe_to_command(r#"{print $0|"sh"}"#));
    }

    #[test]
    fn awk_f_missing_file_asks() {
        let dir = temp_dir("awk-missing");
        let args: Vec<String> = vec!["-f".into(), "missing.awk".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("awk", &args)
        };
        let result = AWK_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Ask(_)));
        cleanup_dir(&dir);
    }
}
