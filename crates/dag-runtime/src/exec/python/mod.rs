//! Python-step executor: embedded RustPython VM (default) or `runc` sandbox.
//!
//! Contract (see `exec/mod.rs` for the shared shape):
//! - `sandbox: in_process` (default) — a FRESH interpreter per step on
//!   `spawn_blocking` (the VM is CPU-bound and sync; module state never
//!   leaks between steps), `sys.stdout`/`sys.stderr` redirected into
//!   `_io.StringIO` buffers, globals `RUN_ID` / `STEP_DIR` / `context`.
//! - `sandbox: runc` — fail-closed: no runc on the node means an Error
//!   outcome, never a silent in-process fallback.
//! - After a clean run, `<step_dir>/output.json` (when the step wrote one)
//!   is parsed into `StepResult::output_json`; a malformed file is an Error
//!   outcome. Writing `output.txt` / `meta.json` is the RUNTIME's job.
//!
//! Stdlib scope note: the workspace pulls `rustpython-vm` with default
//! features only, so the importable set is the VM's core builtin set
//! (`_io`, `itertools`, `posix`, `time`, `_sre`, ...) plus the frozen core
//! modules — `math`/`json`/`os` style pure/CPython-extension stdlib is NOT
//! present (those live in the separate `rustpython-stdlib`/`rustpython-pylib`
//! crates, which are not workspace dependencies).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use opencoder_dag::artifacts::step_dir;
use opencoder_dag::{SandboxMode, StepKind, StepOutcome};
use rustpython_vm::builtins::PyStr;
use rustpython_vm::{Interpreter, PyObjectRef, Settings, VirtualMachine};
use serde_json::Value;

use super::{StepCtx, StepResult};

/// Bounded tail for tracebacks / runc output in `error`.
const ERROR_TAIL_BYTES: usize = 2048;

/// Entry point used by the runtime for `StepKind::Python`.
pub async fn execute_python_step(ctx: &StepCtx) -> StepResult {
    let (code, sandbox) = match &ctx.step.kind {
        StepKind::Python { code, sandbox } => (code.clone(), sandbox.unwrap_or_default()),
        other => return error_result(format!("python executor got a non-python step: {other:?}")),
    };
    match sandbox {
        SandboxMode::InProcess => execute_in_process(ctx, code).await,
        SandboxMode::Runc => execute_runc(ctx, &code).await,
    }
}

// ---------------------------------------------------------------------------
// In-process (embedded VM)
// ---------------------------------------------------------------------------

async fn execute_in_process(ctx: &StepCtx, code: String) -> StepResult {
    let run_id = ctx.run_id.clone();
    let step_name = ctx.step.name.clone();
    let workflow_root = ctx.workflow_root.clone();
    let context = ctx.context();

    // The VM is sync and CPU-bound: run it on the blocking pool. We cannot
    // abort a detached blocking thread — on timeout the thread keeps
    // spinning until its code finishes (or the process exits); there is no
    // cooperative-cancellation hook in the interpreter loop. The `runc`
    // path does not have this limitation: the runner KILLS the container.
    let blocking = tokio::task::spawn_blocking(move || {
        run_vm_step(&run_id, &step_name, &workflow_root, &context, &code)
    });
    let joined = match ctx.step.timeout_secs {
        Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), blocking).await {
            Ok(joined) => joined,
            Err(_elapsed) => return error_result("python step timeout".to_string()),
        },
        None => blocking.await,
    };
    match joined {
        Ok(result) => result,
        Err(join) => error_result(format!("python step panicked: {join}")),
    }
}

/// What one in-process VM run produced (plain data — all PyRefs are dropped
/// inside the interpreter closure before this leaves it).
struct VmOutcome {
    stdout: String,
    stderr: String,
    /// Formatted traceback (REPL-style) when the step code raised.
    traceback: Option<String>,
}

/// Sync, runs entirely inside `spawn_blocking`.
fn run_vm_step(
    run_id: &str,
    step_name: &str,
    workflow_root: &Path,
    context: &Value,
    code: &str,
) -> StepResult {
    let step_dir = match step_dir(workflow_root, run_id, step_name) {
        Ok(dir) => dir,
        Err(err) => return error_result(format!("illegal step path: {err}")),
    };
    if let Err(err) = fs::create_dir_all(&step_dir) {
        return error_result(format!("cannot create step dir: {err}"));
    }

    // Embedded-mode settings. RustPython's default `install_signal_handlers:
    // true` walks every signal 1..NSIG and momentarily sets each to SIG_IGN
    // (its handler-probing trick) — while SIGCHLD is SIG_IGN the kernel
    // auto-reaps any concurrently exiting child, so a runc step child dying
    // in that window is stolen and its `waitpid` fails with ECHILD. It also
    // hijacks SIGINT for the whole host process. An embedded step VM must
    // keep its hands off host signal disposition.
    let mut settings = Settings::default();
    settings.install_signal_handlers = false;
    let outcome = Interpreter::with_init(settings, |_| {}).enter(|vm| {
        // --- Prelude: park sys.stdout/stderr on StringIO buffers. ---
        let original_stdout = vm.sys_module.get_attr("stdout", vm).ok();
        let original_stderr = vm.sys_module.get_attr("stderr", vm).ok();
        let prelude = "import sys\nfrom _io import StringIO\nsys.stdout = StringIO()\nsys.stderr = StringIO()\n";
        if let Err(exc) = vm.run_string(vm.new_scope_with_builtins(), prelude, "<dag-prelude>".to_owned()) {
            let tb = format_exception(vm, &exc);
            restore_streams(vm, original_stdout, original_stderr);
            return VmOutcome {
                stdout: String::new(),
                stderr: String::new(),
                traceback: Some(format!("stdout capture failed: {tb}")),
            };
        }
        let out_buffer = vm.sys_module.get_attr("stdout", vm).ok();
        let err_buffer = vm.sys_module.get_attr("stderr", vm).ok();

        // --- Step code with the documented globals. ---
        let scope = match vm.new_scope_with_main() {
            Ok(scope) => scope,
            Err(exc) => {
                let tb = format_exception(vm, &exc);
                let stdout = read_stringio(vm, &out_buffer);
                let stderr = read_stringio(vm, &err_buffer);
                restore_streams(vm, original_stdout, original_stderr);
                return VmOutcome { stdout, stderr, traceback: Some(tb) };
            }
        };
        let globals = scope.globals.clone();
        let _ = globals.set_item("__name__", vm.ctx.new_str("__main__").into(), vm);
        let _ = globals.set_item("RUN_ID", vm.ctx.new_str(run_id).into(), vm);
        let _ = globals.set_item(
            "STEP_DIR",
            vm.ctx.new_str(step_dir.to_string_lossy().into_owned()).into(),
            vm,
        );
        let _ = globals.set_item("context", json_to_py(context, vm), vm);

        let run = vm.run_string(scope, code, format!("<dag-step:{step_name}>"));
        let traceback = run.err().map(|exc| format_exception(vm, &exc));

        // --- Read the capture buffers, then restore the original streams. ---
        let stdout = read_stringio(vm, &out_buffer);
        let stderr = read_stringio(vm, &err_buffer);
        restore_streams(vm, original_stdout, original_stderr);
        VmOutcome { stdout, stderr, traceback }
    });

    let output_text = render_output_text(&outcome.stdout, &outcome.stderr);
    match outcome.traceback {
        Some(tb) => StepResult {
            outcome: StepOutcome::Error,
            error: Some(tail(&tb, ERROR_TAIL_BYTES)),
            output_text,
            output_json: None,
            session_id: None,
        },
        None => finish_from_output_json(&step_dir, output_text),
    }
}

/// Captured stdout; stderr appended under a separator when non-empty.
fn render_output_text(stdout: &str, stderr: &str) -> String {
    if stderr.trim().is_empty() {
        return stdout.to_string();
    }
    let mut text = String::from(stdout);
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("-- stderr --\n");
    text.push_str(stderr);
    text
}

fn restore_streams(vm: &VirtualMachine, stdout: Option<PyObjectRef>, stderr: Option<PyObjectRef>) {
    if let Some(obj) = stdout {
        let _ = vm.sys_module.set_attr("stdout", obj, vm);
    }
    if let Some(obj) = stderr {
        let _ = vm.sys_module.set_attr("stderr", obj, vm);
    }
}

/// REPL-style `Traceback ... ExcType: message` via the VM's own formatter.
fn format_exception(
    vm: &VirtualMachine,
    exc: &rustpython_vm::PyRef<rustpython_vm::builtins::PyBaseException>,
) -> String {
    let mut buf = String::new();
    let _ = vm.write_exception(&mut buf, exc);
    buf
}

/// `buffer.getvalue()` as Rust text (best-effort: anything unexpected is
/// just an empty capture).
fn read_stringio(vm: &VirtualMachine, buffer: &Option<PyObjectRef>) -> String {
    let Some(buffer) = buffer else {
        return String::new();
    };
    vm.call_method(buffer, "getvalue", ())
        .ok()
        .and_then(|value| value.downcast::<PyStr>().ok())
        .map(|s| AsRef::<str>::as_ref(&s).to_owned())
        .unwrap_or_default()
}

/// serde_json → python objects (null→None, bool, int, float, str, list, dict).
fn json_to_py(value: &Value, vm: &VirtualMachine) -> PyObjectRef {
    match value {
        Value::Null => vm.ctx.none(),
        Value::Bool(b) => vm.ctx.new_bool(*b).into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                vm.ctx.new_int(i).into()
            } else if let Some(u) = n.as_u64() {
                vm.ctx.new_int(u).into()
            } else {
                vm.ctx.new_float(n.as_f64().unwrap_or(f64::NAN)).into()
            }
        }
        Value::String(s) => vm.ctx.new_str(s.as_str()).into(),
        Value::Array(items) => vm
            .ctx
            .new_list(items.iter().map(|item| json_to_py(item, vm)).collect())
            .into(),
        Value::Object(map) => {
            let dict = vm.ctx.new_dict();
            for (key, item) in map {
                let _ = dict.set_item(key.as_str(), json_to_py(item, vm), vm);
            }
            dict.into()
        }
    }
}

// ---------------------------------------------------------------------------
// runc sandbox
// ---------------------------------------------------------------------------

async fn execute_runc(ctx: &StepCtx, code: &str) -> StepResult {
    // Fail-closed: no runc on this node is an error, never a silent
    // in-process fallback (the step explicitly opted OUT of the VM).
    if !crate::sandbox::runc::runc_available() {
        return error_result("runc not installed on this node".to_string());
    }
    let run_root = match opencoder_dag::artifacts::run_root(&ctx.workflow_root, &ctx.run_id) {
        Ok(root) => root,
        Err(err) => return error_result(format!("illegal run path: {err}")),
    };
    let spec = crate::sandbox::oci::BundleSpec {
        run_root: run_root.clone(),
        step_slug: ctx.step.name.clone(),
        code: code.to_string(),
        timeout_hint: ctx.step.timeout_secs,
    };
    // Bundles live next to the run root (never inside it — the run root is
    // the user-visible `/workspace/context` bind).
    let bundle_dir = ctx
        .workflow_root
        .join("bundles")
        .join(&ctx.run_id)
        .join(&ctx.step.name);
    let bundle_dir = match crate::sandbox::oci::write_bundle(&bundle_dir, &spec) {
        Ok(dir) => dir,
        Err(err) => return error_result(format!("cannot build oci bundle: {err:#}")),
    };

    // run_step owns the timeout here: on expiry it KILLS the container and
    // reaps it with `runc delete --force`.
    let container_id = format!("{}-{}", ctx.run_id, ctx.step.name);
    match crate::sandbox::runc::run_step(&bundle_dir, &container_id, ctx.step.timeout_secs).await {
        Ok((0, stdout)) => {
            let step_dir = run_root.join(&ctx.step.name);
            finish_from_output_json(&step_dir, stdout)
        }
        Ok((code, output)) => error_result(format!(
            "runc step exited with {code}:\n{}",
            tail(&output, ERROR_TAIL_BYTES)
        )),
        Err(err) => error_result(err.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Success path: parse the step's optional `output.json`.
fn finish_from_output_json(step_dir: &Path, output_text: String) -> StepResult {
    let json_path = step_dir.join("output.json");
    if !json_path.is_file() {
        return done_result(output_text, None);
    }
    let text = match fs::read_to_string(&json_path) {
        Ok(text) => text,
        Err(err) => return error_result(format!("cannot read output.json: {err}")),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => done_result(output_text, Some(value)),
        Err(err) => error_result(format!(
            "output.json is not valid JSON ({err}); written to {}",
            json_path.display()
        )),
    }
}

fn done_result(output_text: String, output_json: Option<Value>) -> StepResult {
    StepResult {
        outcome: StepOutcome::Done,
        error: None,
        output_text,
        output_json,
        session_id: None,
    }
}

fn error_result(error: String) -> StepResult {
    StepResult {
        outcome: StepOutcome::Error,
        error: Some(error),
        output_text: String::new(),
        output_json: None,
        session_id: None,
    }
}

/// Last `max_bytes` of `text` on a char boundary, marked when truncated.
fn tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    format!("...\n{}", &text[start..])
}

#[allow(dead_code)] // path bookkeeping used by debug tooling
fn step_dir_of(workflow_root: &Path, run_id: &str, step: &str) -> Option<PathBuf> {
    step_dir(workflow_root, run_id, step).ok()
}

#[cfg(test)]
mod tests;
