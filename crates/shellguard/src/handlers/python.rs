//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use super::{
    first_positional, get_flag_value, has_flag, is_sole_help_flag, Classification, Handler,
    HandlerContext,
};
use crate::python_safety::is_python_source_safe;
use crate::verdict::AllowReason;

pub(crate) static PYTHON_HANDLER: PythonHandler = PythonHandler;

pub(crate) struct PythonHandler;

/// Stdlib modules `-m` may run: they print and exit, taking no code from argv.
const SAFE_MODULES: &[&str] = &["calendar", "json.tool", "this", "antigravity"];

impl Handler for PythonHandler {
    fn commands(&self) -> &[&str] {
        &[
            "python",
            "python3",
            "python3.8",
            "python3.9",
            "python3.10",
            "python3.11",
            "python3.12",
            "python3.13",
            "python3.14",
        ]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if is_sole_help_flag(ctx.args, &["--version", "-V", "-VV", "--help", "-h"]) {
            return Classification::Allow(AllowReason::handler("python version/help"));
        }

        // -c inline code — analyze source for dangerous patterns
        if let Some(source) = get_flag_value(ctx.args, &["-c"]) {
            return if is_python_source_safe(&source) {
                Classification::Allow(AllowReason::handler("python -c (safe inline code)"))
            } else {
                Classification::Ask("python -c (potentially dangerous code)".into())
            };
        }

        // -m module
        if has_flag(ctx.args, &["-m"]) {
            let module = ctx
                .args
                .iter()
                .skip_while(|a| a.as_str() != "-m")
                .nth(1)
                .map_or("", String::as_str);
            return if SAFE_MODULES.contains(&module) {
                Classification::Allow(AllowReason::handler(format!("python -m {module}")))
            } else {
                Classification::Ask(format!("python -m {module}"))
            };
        }

        // -i interactive
        if has_flag(ctx.args, &["-i"]) {
            return Classification::Ask("python -i (interactive)".into());
        }

        // No args = interactive
        if ctx.args.is_empty() {
            return Classification::Ask("python (interactive)".into());
        }

        // Script execution — try to read and analyze the file
        let script = first_positional(ctx.args).unwrap_or("");
        if let Some(source) = ctx.read_file(script) {
            return if is_python_source_safe(&source) {
                Classification::Allow(AllowReason::handler(format!(
                    "python {script} (safe script)"
                )))
            } else {
                Classification::Ask(format!("python {script} (potentially dangerous)"))
            };
        }
        Classification::Ask("python script execution".into())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::handlers::test_support::{temp_dir, write_file};

    // Command->decision cases (version/-c inline/-m/no-args/missing-script) are
    // covered by rippy's command catalog (not ported). The remaining
    // tests exercise read_file, which the catalog cannot inject.
    #[test]
    fn script_file_safe_allows() {
        let dir = temp_dir("fs");
        write_file(&dir, "safe.py", "import json\nprint(json.dumps({}))");
        let args = vec!["safe.py".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("python", &args)
        };
        let result = PYTHON_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Allow(_)));
    }

    #[test]
    fn script_file_dangerous_asks() {
        let dir = temp_dir("fs");
        write_file(&dir, "evil.py", "import os\nos.system('rm -rf /')");
        let args = vec!["evil.py".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("python", &args)
        };
        let result = PYTHON_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Ask(_)));
    }

    #[test]
    fn script_file_missing_asks() {
        let dir = temp_dir("fs");
        let args = vec!["missing.py".into()];
        let ctx = HandlerContext {
            working_directory: &dir,
            ..HandlerContext::test("python", &args)
        };
        let result = PYTHON_HANDLER.classify(&ctx);
        assert!(matches!(result, Classification::Ask(_)));
    }
}
