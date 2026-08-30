//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    Classification, Handler, HandlerContext, first_positional, get_flag_values,
    has_clustered_short_flag, is_sole_help_flag,
};
use crate::perl_safety::is_perl_source_safe;
use crate::verdict::AllowReason;

pub(crate) static PERL_HANDLER: PerlHandler = PerlHandler;

/// Perl short flags that take a value glued to the cluster (`-e CODE`,
/// `-Ilib`, `-MModule`, `-Fpat`, `-xdir`): the cluster scan must stop there so
/// a glued value is never read as further flags. Note `-I` is uppercase — the
/// in-place marker is the lowercase `i`.
const PERL_VALUE_FLAGS: &[char] = &['e', 'E', 'I', 'M', 'm', 'F', 'x'];

pub(crate) struct PerlHandler;

impl Handler for PerlHandler {
    fn commands(&self) -> &[&str] {
        &["perl"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--version", "-v", "--help", "-h"]) {
            return Classification::Allow(AllowReason::handler("perl version/help"));
        }

        // Clustered short flags (`-pi`, `-ipe`, `-pi.bak`) can hide an in-place
        // `-i`; the `-e` harvest below would then bless the script as harmless
        // inline code while it rewrites files. Checked before that harvest.
        if has_clustered_short_flag(ctx.args, 'i', PERL_VALUE_FLAGS) {
            return Classification::Ask("perl in-place edit (-i)".into());
        }

        // -e / -E inline code — Perl concatenates every fragment with "\n" at
        // runtime, so all occurrences must be analyzed together, not just the first.
        let fragments = get_flag_values(ctx.args, &["-e", "-E"]);
        if !fragments.is_empty() {
            let source = fragments.join("\n");
            return if is_perl_source_safe(&source) {
                Classification::Allow(AllowReason::handler("perl -e (safe inline code)"))
            } else {
                Classification::Ask("perl -e (potentially dangerous code)".into())
            };
        }

        // No args = reads from stdin
        if ctx.args.is_empty() {
            return Classification::Ask("perl (reads stdin)".into());
        }

        // Script file execution — try to read and analyze
        let script = first_positional(ctx.args).unwrap_or("");
        if let Some(source) = ctx.read_file(script) {
            return if is_perl_source_safe(&source) {
                Classification::Allow(AllowReason::handler(format!("perl {script} (safe script)")))
            } else {
                Classification::Ask(format!("perl {script} (potentially dangerous)"))
            };
        }
        Classification::Ask("perl script execution".into())
    }

}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::handlers::test_support::{temp_dir, write_file};
    use std::path::Path;

    /// In-place Ask must fire regardless of the operand path, but the test
    /// context points at a non-release cwd (`HandlerContext::test` defaults to
    /// the released `/tmp`).
    fn non_release_ctx(args: &[String]) -> HandlerContext<'_> {
        HandlerContext {
            working_directory: Path::new("/home/u/project"),
            ..HandlerContext::test("perl", args)
        }
    }

    fn asks(args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(PERL_HANDLER.classify(&non_release_ctx(&args)), Classification::Ask(_)),
            "expected Ask for {args:?}"
        );
    }

    fn allows(args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(PERL_HANDLER.classify(&non_release_ctx(&args)), Classification::Allow(_)),
            "expected Allow for {args:?}"
        );
    }

    #[test]
    fn clustered_in_place_flags_ask() {
        asks(&["-pi", "-e", "s/a/b/", "f"]); // classic -pi rewrite
        asks(&["-i", "-pe", "s/a/b/", "f"]); // split form
        asks(&["-ipe", "s/a/b/", "f"]); // in-place glued to the cluster
        asks(&["-pi.bak", "-e", "s/a/b/", "f"]); // glued backup suffix
    }

    #[test]
    fn safe_inline_code_still_allows() {
        allows(&["-e", "print 1"]);
        allows(&["-E", "say 'hi'"]);
        // The letter `i` inside an -e VALUE (separate token) is not a flag.
        allows(&["-e", "print \"i\";"]);
        // `-Ilib` / `-MModule` carry glued values; uppercase `I` is not `-i`.
        allows(&["-Ilib", "-e", "print 1"]);
        allows(&["-MList::Util", "-e", "print max(1, 2)"]);
    }

    // Handler-level safe/dangerous inline distinction. NOTE: the full pipeline's

    // Handler-level safe-inline Allow. The full pipeline Asks (catch-all
    // `command=perl` rule); catalog covers the pipeline decision.
    #[test]
    fn e_safe_print_allows() {
        let args = vec!["-e".into(), "print 'hello\\n'".into()];
        assert!(matches!(
            PERL_HANDLER.classify(&HandlerContext::test("perl", &args)),
            Classification::Allow(_)
        ));
    }

    // Handler-level danger arm: `-e`/`-E` inline dangerous code must Ask. The catalog's
    // isolated stdlib catch-all Asks for any `perl`, masking this arm at the pipeline
    // level, so the safety-critical danger->Ask direction is only observable here.
    #[test]
    fn e_dangerous_system_asks() {
        let args = vec!["-e".into(), "system('rm -rf /')".into()];
        assert!(matches!(
            PERL_HANDLER.classify(&HandlerContext::test("perl", &args)),
            Classification::Ask(_)
        ));
    }

    // Every -e fragment is concatenated for analysis, not just the first, so a
    // dangerous call hidden in a later fragment must still Ask.
    #[test]
    fn second_e_fragment_dangerous_asks() {
        let args = vec![
            "-e".into(),
            "1".into(),
            "-e".into(),
            "system(\"id\")".into(),
        ];
        assert!(matches!(
            PERL_HANDLER.classify(&HandlerContext::test("perl", &args)),
            Classification::Ask(_)
        ));
    }

    #[test]
    fn upper_e_dangerous_system_asks() {
        let args = vec!["-E".into(), "system('rm -rf /')".into()];
        assert!(matches!(
            PERL_HANDLER.classify(&HandlerContext::test("perl", &args)),
            Classification::Ask(_)
        ));
    }

    #[test]
    fn script_file_safe_allows() {
        let dir = temp_dir("fs");
        write_file(&dir, "safe.pl", "print 'hello\\n'");
        let args = vec!["safe.pl".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("perl", &args)
        };
        assert!(matches!(
            PERL_HANDLER.classify(&ctx),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn script_file_dangerous_asks() {
        let dir = temp_dir("fs");
        write_file(&dir, "evil.pl", "system('rm -rf /')");
        let args = vec!["evil.pl".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("perl", &args)
        };
        assert!(matches!(
            PERL_HANDLER.classify(&ctx),
            Classification::Ask(_)
        ));
    }
}
