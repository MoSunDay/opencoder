//! Archive utilities: `tar`, `gzip`/`gunzip`, `unzip` and `7z`.
//!
//! Program-spawning tar options ask; listing modes and inert compression
//! operations allow.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    has_flag, is_sole_help_flag, Classification, Handler, HandlerContext, SubcommandHandler,
};
use crate::verdict::AllowReason;

const TAR_PROGRAM_EXEC_LONG: &[&str] = &[
    "--use-compress-program",
    "--to-command",
    "--checkpoint-action",
    "--rmt-command",
    "--info-script",
    "--new-volume-script",
];

/// Long options that are exact spellings of inert tar options while also being a
/// proper prefix of a [`TAR_PROGRAM_EXEC_LONG`] entry.
///
/// `getopt_long` resolves an exact match before it considers prefixes, so
/// `--checkpoint=1000` is the progress counter, not an abbreviation of
/// `--checkpoint-action`.
const TAR_INERT_EXEC_PREFIXES: &[&str] = &["--checkpoint"];

/// tar short options that spawn an external program.
const TAR_EXEC_SHORT: &[char] = &['I', 'F'];

/// tar short options that take a value, which getopt reads from the rest of the
/// cluster when it is non-empty (`-tfFoo.tar` selects the file `Foo.tar`).
///
/// Scanning a cluster stops at the first of these: everything after it is a
/// value, so letters there are not options.
const TAR_SHORT_WITH_VALUE: &[char] = &[
    'b', 'C', 'f', 'F', 'g', 'H', 'I', 'K', 'L', 'N', 'T', 'V', 'X',
];

// tar

pub(crate) static TAR_HANDLER: TarHandler = TarHandler;

pub(crate) struct TarHandler;

impl Handler for TarHandler {
    fn commands(&self) -> &[&str] {
        &["tar"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        let mut class = Self::classify_archive_op(ctx);
        // A spawned program is a risk on top of the archive operation, not instead of it (#198).
        for program in Self::to_command_programs(ctx.args) {
            class = Classification::RecurseAtLeast(program, Box::new(class));
        }
        class
    }
}

impl TarHandler {
    /// The verdict for the archive operation itself, independent of any program
    /// `--to-command` spawns.
    fn classify_archive_op(ctx: &HandlerContext) -> Classification {
        if ctx.args.iter().any(|a| runs_external_program(a)) {
            return Classification::Ask("tar (runs external program)".into());
        }
        if has_flag(ctx.args, &["-t", "--list"]) {
            return Classification::Allow(AllowReason::handler("tar (list)"));
        }
        Classification::Ask("tar (create/extract)".into())
    }

    /// The program handed to each `--to-command`, in both the spaced and the
    /// `=`-glued spelling and under any prefix abbreviation.
    ///
    /// Every occurrence is returned, not the first: tar's *last* `--to-command`
    /// is the one it runs, and an earlier one has already run for the members
    /// before it, so judging one and ignoring the rest judges the wrong program.
    fn to_command_programs(args: &[String]) -> Vec<String> {
        let mut programs = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let (name, glued) = split_long_option(&args[i]);
            if name.is_some_and(|n| "--to-command".starts_with(n)) {
                if let Some(value) = glued {
                    programs.push(value.to_owned());
                } else if let Some(value) = args.get(i + 1) {
                    programs.push(value.clone());
                    i += 1;
                }
            }
            i += 1;
        }
        programs
    }
}

/// Splits a long option into its name and its `=`-glued value. The name is
/// `None` for anything that is not a long option, bare `--` included.
fn split_long_option(arg: &str) -> (Option<&str>, Option<&str>) {
    if !arg.starts_with("--") || arg.len() == 2 {
        return (None, None);
    }
    arg.split_once('=')
        .map_or((Some(arg), None), |(name, value)| (Some(name), Some(value)))
}

/// Whether a token selects one of tar's program-spawning options.
fn runs_external_program(arg: &str) -> bool {
    split_long_option(arg)
        .0
        .map_or_else(|| cluster_has_exec_short(arg), is_exec_long_option)
}

/// GNU tar (argp/`getopt_long`) accepts *any prefix* of a long option, so matching
/// exact spellings cannot work: `--use-c` really does select
/// `--use-compress-program` (#198). A spelling is an exec option when a known
/// exec option starts with it. An ambiguous prefix makes real tar exit with an
/// error, so treating it as the exec option it could name costs nothing.
fn is_exec_long_option(name: &str) -> bool {
    !TAR_INERT_EXEC_PREFIXES.contains(&name)
        && TAR_PROGRAM_EXEC_LONG.iter().any(|f| f.starts_with(name))
}

/// Scans a short-option cluster (`-xIf`, `-Ish`) for an exec option.
fn cluster_has_exec_short(arg: &str) -> bool {
    let Some(cluster) = arg.strip_prefix('-') else {
        return false;
    };
    for c in cluster.chars() {
        if TAR_EXEC_SHORT.contains(&c) {
            return true;
        }
        // Everything after a value-taking option is that value, not more options.
        if TAR_SHORT_WITH_VALUE.contains(&c) {
            return false;
        }
    }
    false
}

// gzip

pub(crate) static GZIP_HANDLER: SubcommandHandler = SubcommandHandler::new(
    &["gzip", "gunzip"],
    &["--stdout", "-c", "--list", "-l", "--test", "-t"],
    &[],
    "gzip",
);

// unzip
//
// Unlike 7z, unzip takes its mode as a FLAG (`-l` list, `-t` test), not a bare
// subcommand verb — `unzip l archive.zip` is not a real invocation. See #190.

pub(crate) static UNZIP_HANDLER: UnzipHandler = UnzipHandler;

pub(crate) struct UnzipHandler;

impl Handler for UnzipHandler {
    fn commands(&self) -> &[&str] {
        &["unzip"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--help", "-h", "--version", "-V"]) {
            return Classification::Allow(AllowReason::handler("unzip help/version"));
        }
        if has_leading_unzip_mode_flag(ctx.args) {
            return Classification::Allow(AllowReason::handler("unzip (list/test)"));
        }
        Classification::Ask("unzip (extract)".into())
    }
}

/// unzip mode letters that make the invocation read-only: list, test, verbose
/// list and zipinfo mode.
const UNZIP_MODE_LETTERS: &[char] = &['l', 't', 'v', 'Z'];

/// unzip letters whose value may be glued to them (`-dlogs`, `-Psecret`), so the
/// rest of the token is data rather than more clustered flags.
const UNZIP_VALUE_LETTERS: &[char] = &['d', 'O', 'I', 'P'];

/// Whether a read-only mode flag appears in the option run that precedes the
/// archive operand, which is the only place unzip treats it as an option: words
/// after the archive are member filespecs, so a trailing `-l` is consumed as a
/// (non-matching) member name while the rest of argv still extracts (#190).
fn has_leading_unzip_mode_flag(args: &[String]) -> bool {
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if arg.starts_with("--") {
            return false; // unknown long option: fail closed
        }
        let Some(letters) = arg.strip_prefix('-') else {
            return false; // the archive operand ends the option run
        };
        for (i, ch) in letters.char_indices() {
            if UNZIP_MODE_LETTERS.contains(&ch) {
                return true;
            }
            if UNZIP_VALUE_LETTERS.contains(&ch) {
                skip_value = i + ch.len_utf8() == letters.len();
                break;
            }
        }
    }
    false
}

// 7z / 7za / 7zr / 7zz — bare subcommand verbs (`7z l archive.7z`), unlike unzip.

pub(crate) static SEVENZIP_HANDLER: SubcommandHandler = SubcommandHandler::new(
    &["7z", "7za", "7zr", "7zz"],
    &["l", "t"],                // list and test
    &["x", "e", "a", "d", "u"], // extract, add, delete, update
    "7z",
);
