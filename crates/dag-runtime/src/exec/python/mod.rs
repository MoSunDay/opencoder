//! Python-step executor: embedded RustPython VM (default) or `runc` sandbox.
//!
//! Contract (see `exec/mod.rs` for the shared shape):
//! - `sandbox: in_process` (default) — a FRESH interpreter per step on
//!   a killable internal agent child (the legacy wire name is retained;
//!   interpreter state never leaks between steps), `sys.stdout`/`sys.stderr` redirected into
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

use opencoder_dag::artifacts::step_dir;
use opencoder_dag::{SandboxMode, StepKind, StepOutcome};
use rustpython_vm::builtins::PyStr;
use rustpython_vm::{Interpreter, PyObjectRef, Settings, VirtualMachine};
use serde_json::Value;

use super::{StepCtx, StepResult};

/// Bounded tail for tracebacks / runc output in `error`.
const ERROR_TAIL_BYTES: usize = 2048;

/// Entry point used by the runtime for `StepKind::Python`.
mod process;
pub use process::worker_main;
use tokio_util::sync::CancellationToken;

pub async fn execute_python_step(ctx: &StepCtx) -> StepResult {
    execute_python_step_cancellable(ctx, CancellationToken::new()).await
}

pub async fn execute_python_step_cancellable(
    ctx: &StepCtx,
    cancel: CancellationToken,
) -> StepResult {
    let (code, sandbox) = match &ctx.step.kind {
        StepKind::Python { code, sandbox } => (code.clone(), sandbox.unwrap_or_default()),
        other => return error_result(format!("python executor got a non-python step: {other:?}")),
    };
    match sandbox {
        SandboxMode::InProcess => process::execute(ctx, code, cancel).await,
        SandboxMode::Runc => execute_runc(ctx, &code, cancel).await,
    }
}

// ---------------------------------------------------------------------------
// In-process (embedded VM)
// ---------------------------------------------------------------------------

/// What one in-process VM run produced (plain data — all PyRefs are dropped
/// inside the interpreter closure before this leaves it).
struct VmOutcome {
    stdout: String,
    stderr: String,
    /// Formatted traceback (REPL-style) when the step code raised.
    traceback: Option<String>,
}

/// Sync, runs only in the dedicated VM child process.
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
        // --- Prelude: park sys.stdout/stderr on byte-counted buffers. ---
        let original_stdout = vm.sys_module.get_attr("stdout", vm).ok();
        let original_stderr = vm.sys_module.get_attr("stderr", vm).ok();
        let prelude = format!(
            "import sys\n\
             from _io import StringIO\n\
             class _OpenCoderOutput:\n\
             \x20   def __init__(self, name):\n\
             \x20       self._name = name\n\
             \x20       self._bytes = 0\n\
             \x20       self._stream = StringIO()\n\
             \x20   def write(self, value):\n\
             \x20       size = len(value.encode('utf-8'))\n\
             \x20       if self._bytes + size > {limit}:\n\
             \x20           raise RuntimeError('output_limit_exceeded: %s exceeds {limit} bytes' % self._name)\n\
             \x20       self._bytes += size\n\
             \x20       return self._stream.write(value)\n\
             \x20   def flush(self):\n\
             \x20       return None\n\
             \x20   def getvalue(self):\n\
             \x20       return self._stream.getvalue()\n\
             sys.stdout = _OpenCoderOutput('python stdout')\n\
             sys.stderr = _OpenCoderOutput('python stderr')\n",
            limit = crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,
        );
        if let Err(exc) = vm.run_string(
            vm.new_scope_with_builtins(),
            &prelude,
            "<dag-prelude>".to_owned(),
        ) {
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
            error: Some(output_limit_message(&tb).unwrap_or_else(|| tail(&tb, ERROR_TAIL_BYTES))),
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

fn output_limit_message(traceback: &str) -> Option<String> {
    const MARKER: &str = "output_limit_exceeded:";
    let start = traceback.rfind(MARKER)?;
    Some(traceback[start..].lines().next()?.trim().to_string())
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

async fn execute_runc(ctx: &StepCtx, code: &str, cancel: CancellationToken) -> StepResult {
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
    // Copying a provisioned interpreter can take time; keep node heartbeats
    // and unrelated executions responsive while preparing the private tree.
    let prepared =
        tokio::task::spawn_blocking(move || crate::sandbox::oci::write_bundle(&bundle_dir, &spec))
            .await;
    let bundle_dir = match prepared {
        Ok(Ok(dir)) => dir,
        Ok(Err(err)) => return error_result(format!("cannot build oci bundle: {err:#}")),
        Err(err) => return error_result(format!("oci bundle preparation failed: {err}")),
    };

    // run_step owns the timeout here: on expiry it KILLS the container and
    // reaps it with `runc delete --force`.
    let container_id = format!("{}-{}", ctx.run_id, ctx.step.name);
    match crate::sandbox::runc::run_step_cancellable(
        &bundle_dir,
        &container_id,
        ctx.step.timeout_secs,
        cancel.clone(),
    )
    .await
    {
        Ok((0, stdout)) => {
            let step_dir = run_root.join(&ctx.step.name);
            finish_from_output_json(&step_dir, stdout)
        }
        Ok((code, output)) => error_result(format!(
            "runc step exited with {code}:\n{}",
            tail(&output, ERROR_TAIL_BYTES)
        )),
        Err(err) if cancel.is_cancelled() => StepResult {
            outcome: StepOutcome::Cancelled,
            ..error_result(err.to_string())
        },
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
    let bytes = match crate::sandbox::output_limit::read_file_bounded(
        &json_path,
        "output.json",
        crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
    ) {
        Ok(bytes) => bytes,
        Err(err) => return error_result(format!("cannot read output.json: {err}")),
    };
    match serde_json::from_slice::<Value>(&bytes) {
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
