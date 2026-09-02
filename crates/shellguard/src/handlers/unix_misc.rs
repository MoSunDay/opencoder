//! Assorted unix utilities: `wget`, `mktemp`, `tee`, `sort`, `open`, `yq`,
//! `dos2unix`/`unix2dos`.
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    has_flag, has_flag_or_prefixed, is_sole_help_flag, positional_args, Classification, Handler,
    HandlerContext,
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
