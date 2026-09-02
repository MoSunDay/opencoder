//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    first_positional, get_flag_value, has_clustered_short_flag, is_sole_help_flag, Classification,
    Handler, HandlerContext,
};
use crate::ruby_safety::is_ruby_source_safe;
use crate::verdict::AllowReason;

pub(crate) static RUBY_HANDLER: RubyHandler = RubyHandler;

/// Ruby short flags that take a value glued to the cluster (`-e CODE`,
/// `-Eenc`, `-Idir`, `-rlib`, `-Fpat`, `-xdir`, `-Cdir`, `-Kcode`): the cluster
/// scan must stop there so a glued value is never read as further flags. Note
/// `-I` is uppercase — the in-place marker is the lowercase `i`.
const RUBY_VALUE_FLAGS: &[char] = &['e', 'E', 'I', 'r', 'F', 'x', 'C', 'K'];

pub(crate) struct RubyHandler;

impl Handler for RubyHandler {
    fn commands(&self) -> &[&str] {
        &["ruby", "irb"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--version", "-v", "--help", "-h"]) {
            return Classification::Allow(AllowReason::handler(format!(
                "{} version/help",
                ctx.command_name
            )));
        }

        if ctx.command_name == "irb" {
            return Classification::Ask("irb (interactive)".into());
        }

        // Clustered short flags (`-pi`, `-ipe`, `-pi.bak`) can hide an in-place
        // `-i`; the `-e` analysis below would then bless the script as harmless
        // inline code while it rewrites files. Checked before that analysis.
        if has_clustered_short_flag(ctx.args, 'i', RUBY_VALUE_FLAGS) {
            return Classification::Ask("ruby in-place edit (-i)".into());
        }

        // -e inline code: analyze source for dangerous patterns.
        if let Some(source) = get_flag_value(ctx.args, &["-e"]) {
            return if is_ruby_source_safe(&source) {
                Classification::Allow(AllowReason::handler("ruby -e (safe inline code)"))
            } else {
                Classification::Ask("ruby -e (potentially dangerous code)".into())
            };
        }

        if ctx.args.is_empty() {
            return Classification::Ask("ruby (interactive)".into());
        }

        let script = first_positional(ctx.args).unwrap_or("");
        if let Some(source) = ctx.read_file(script) {
            return if is_ruby_source_safe(&source) {
                Classification::Allow(AllowReason::handler(format!("ruby {script} (safe script)")))
            } else {
                Classification::Ask(format!("ruby {script} (potentially dangerous)"))
            };
        }
        Classification::Ask("ruby script execution".into())
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
            ..HandlerContext::test("ruby", args)
        }
    }

    fn asks(args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(
                RUBY_HANDLER.classify(&non_release_ctx(&args)),
                Classification::Ask(_)
            ),
            "expected Ask for {args:?}"
        );
    }

    fn allows(args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            matches!(
                RUBY_HANDLER.classify(&non_release_ctx(&args)),
                Classification::Allow(_)
            ),
            "expected Allow for {args:?}"
        );
    }

    #[test]
    fn clustered_in_place_flags_ask() {
        asks(&["-pi", "-e", "puts 1", "f"]); // classic -pi rewrite
        asks(&["-i", "-pe", "puts 1", "f"]); // split form
        asks(&["-i.bak", "-pe", "puts 1", "f"]); // glued backup suffix
    }

    #[test]
    fn safe_inline_code_still_allows() {
        allows(&["-e", "puts 1"]);
        // The letter `i` inside an -e VALUE (separate token) is not a flag.
        allows(&["-e", "puts 'i'"]);
        // `-Ilib` carries a glued value; uppercase `I` is not `-i`.
        allows(&["-Ilib", "-e", "puts 1"]);
    }

    // Handler-level safe/dangerous inline distinction. NOTE: the full pipeline's

    // Handler-level safe/dangerous inline distinction. NOTE: the full pipeline's
    // isolated stdlib config has a catch-all `command=ruby` rule that Asks, so the
    // safe-inline Allow is only observable at the handler level here — the catalog
    // covers the pipeline's fail-closed Ask. See rippy's command catalog (not ported).
    #[test]
    fn e_safe_puts_allows() {
        let args = vec!["-e".into(), "puts 'hello'".into()];
        assert!(matches!(
            RUBY_HANDLER.classify(&HandlerContext::test("ruby", &args)),
            Classification::Allow(_)
        ));
    }

    // Handler-level danger arm: `-e` inline dangerous code must Ask. The catalog's
    // isolated stdlib catch-all Asks for any `ruby`, masking this arm at the pipeline
    // level, so the safety-critical danger->Ask direction is only observable here.
    #[test]
    fn e_dangerous_system_asks() {
        let args = vec!["-e".into(), "system('rm -rf /')".into()];
        assert!(matches!(
            RUBY_HANDLER.classify(&HandlerContext::test("ruby", &args)),
            Classification::Ask(_)
        ));
    }

    #[test]
    fn script_file_safe_allows() {
        let dir = temp_dir("fs");
        write_file(&dir, "safe.rb", "puts 'hello'");
        let args = vec!["safe.rb".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("ruby", &args)
        };
        assert!(matches!(
            RUBY_HANDLER.classify(&ctx),
            Classification::Allow(_)
        ));
    }

    #[test]
    fn script_file_dangerous_asks() {
        let dir = temp_dir("fs");
        write_file(&dir, "evil.rb", "system('rm -rf /')");
        let args = vec!["evil.rb".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("ruby", &args)
        };
        assert!(matches!(
            RUBY_HANDLER.classify(&ctx),
            Classification::Ask(_)
        ));
    }
}
