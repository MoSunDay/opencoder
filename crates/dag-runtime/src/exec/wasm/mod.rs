//! Wasm-step executor: embedded wasm runtime (wasmtime, default) or `runc`
//! sandbox.
//!
//! Contract (see `exec/mod.rs` for the shared shape):
//! - `command` is the launch command: `"<module.wasm> [args...]"`,
//!   whitespace-split. The module path is RELATIVE and must be confined
//!   (no absolute paths, no `..` components). Lookup: the run's context
//!   root (`<workflow_root>/<run_id>`, guest-visible at
//!   `/workspace/context` in both sandbox modes) first, then the shared
//!   module library at `<workflow_root>/_modules/` (a module found there
//!   is copied into the run tree for `sandbox: runc` so the guest can see
//!   it).
//! - The upstream `context` is delivered as a `context.json` file written
//!   to `<run_root>/<step>/context.json`; its guest path is passed via the
//!   `OPENCODER_STEP_CONTEXT` env variable (plus `OPENCODER_RUN_ID` /
//!   `OPENCODER_STEP_DIR`). This file contract replaces the retired
//!   RustPython `context` global injection.
//! - After a clean run, `<run_root>/<step>/output.json` (when the module
//!   wrote one) is parsed into `StepResult::output_json`; a malformed file
//!   is an Error outcome. Writing `output.txt` / `meta.json` is the
//!   RUNTIME's job.
//! - `sandbox: runc` — fail-closed: no runc on the node means an Error
//!   outcome, never a silent in-process fallback.

mod in_process;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use opencoder_dag::{SandboxMode, StepKind, StepOutcome};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{StepCtx, StepResult};

/// Bounded tail for tracebacks / runc output in `error`.
const ERROR_TAIL_BYTES: usize = 2048;

/// Guest-visible mount point of the run root in BOTH sandbox modes.
pub(crate) const CONTEXT_MOUNT: &str = "/workspace/context";

/// Split a step `command` into argv tokens: whitespace-separated, no
/// quoting/escape handling (workflow specs pass module paths and plain
/// flags). Returns every non-empty token.
pub(crate) fn split_command(command: &str) -> Vec<String> {
    command.split_whitespace().map(str::to_string).collect()
}

/// Env pairs injected into the wasm execution environment (in-process
/// WasiCtx or the runc container process).
pub(crate) fn step_env(ctx: &StepCtx) -> Vec<(String, String)> {
    vec![
        ("OPENCODER_RUN_ID".into(), ctx.run_id.clone()),
        (
            "OPENCODER_STEP_DIR".into(),
            format!("{}/{}/", CONTEXT_MOUNT, ctx.step.name),
        ),
        (
            "OPENCODER_STEP_CONTEXT".into(),
            format!("{}/{}/context.json", CONTEXT_MOUNT, ctx.step.name),
        ),
    ]
}

/// Resolve the module token to a host path under the run root. Confined:
/// absolute paths and `..` components are rejected before any filesystem
/// work happens (the in-process runtime would otherwise follow them
/// anywhere on the node).
pub(crate) fn module_host_path(
    run_root: &std::path::Path,
    module_token: &str,
) -> Result<PathBuf, String> {
    let rel = std::path::Path::new(module_token);
    if rel.is_absolute() {
        return Err(format!(
            "module path must be relative to {CONTEXT_MOUNT}, got {module_token:?}"
        ));
    }
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "module path must not contain '..': {module_token:?}"
        ));
    }
    Ok(run_root.join(rel))
}

/// Resolve the module token to an existing host file: the run's context
/// root first, then the node's shared module library (same confinement
/// rules). Errors mention both roots when a library is configured.
pub(crate) fn resolve_module(
    run_root: &std::path::Path,
    library: Option<&std::path::Path>,
    module_token: &str,
) -> Result<PathBuf, String> {
    let direct = module_host_path(run_root, module_token)?;
    if direct.is_file() {
        return Ok(direct);
    }
    if let Some(lib) = library {
        let staged = module_host_path(lib, module_token)?;
        if staged.is_file() {
            return Ok(staged);
        }
    }
    match library {
        Some(lib) => Err(format!(
            "wasm module not found under the run context root {} or the module library {}: {}",
            run_root.display(),
            lib.join(module_token).display(),
            module_token
        )),
        None => Err(format!(
            "wasm module not found under the run context root: {}",
            direct.display()
        )),
    }
}

/// Write the upstream `context.json` artifact into the step dir and create
/// the dir. Best-effort caller error mapping happens at the call site.
pub(crate) fn write_context_json(ctx: &StepCtx) -> Result<(), String> {
    // `artifacts::step_dir` takes the WORKFLOW root; join from there so the
    // run id is not duplicated.
    let dir = opencoder_dag::artifacts::step_dir(&ctx.workflow_root, &ctx.run_id, &ctx.step.name)
        .map_err(|e| format!("illegal step path: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create step dir: {e}"))?;
    let pretty = serde_json::to_vec_pretty(&ctx.context()).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("context.json"), pretty).map_err(|e| e.to_string())
}

/// Entry point used by the runtime for `StepKind::Wasm`.
pub async fn execute_wasm_step(ctx: &StepCtx) -> StepResult {
    execute_wasm_step_cancellable(ctx, CancellationToken::new()).await
}

pub async fn execute_wasm_step_cancellable(ctx: &StepCtx, cancel: CancellationToken) -> StepResult {
    let (command, sandbox) = match &ctx.step.kind {
        StepKind::Wasm { command, sandbox } => (command.clone(), sandbox.unwrap_or_default()),
        _ => return error_result("non-wasm step dispatched to the wasm executor".into()),
    };
    let tokens = split_command(&command);
    if tokens.is_empty() {
        return error_result("wasm step has an empty command".into());
    }
    let run_root = match opencoder_dag::artifacts::run_root(&ctx.workflow_root, &ctx.run_id) {
        Ok(root) => root,
        Err(e) => return error_result(format!("illegal run path: {e}")),
    };
    if let Err(e) = write_context_json(ctx) {
        return error_result(e);
    }
    // Shared module library: `<workflow_root>/_modules` (sibling of the
    // run dirs; the node's dag kind root or a project's local root).
    let module_library = ctx.workflow_root.join("_modules");
    match sandbox {
        SandboxMode::InProcess => {
            in_process::execute(
                ctx,
                &run_root,
                Some(&module_library),
                &tokens,
                ctx.step.timeout_secs,
                cancel,
            )
            .await
        }
        SandboxMode::Runc => run_runc(ctx, &run_root, Some(&module_library), &tokens, cancel).await,
    }
}

/// `sandbox: runc`: private OCI bundle + `wasmtime run` inside the
/// container. Fail-closed: no runc binary means an Error outcome.
async fn run_runc(
    ctx: &StepCtx,
    run_root: &std::path::Path,
    module_library: Option<&std::path::Path>,
    tokens: &[String],
    cancel: CancellationToken,
) -> StepResult {
    if !crate::sandbox::runc::runc_available() {
        return error_result(
            "runc executable unavailable for requested wasm sandbox (fail-closed; \
             in_process is the default)"
                .into(),
        );
    }
    let module_path = match resolve_module(run_root, module_library, &tokens[0]) {
        Ok(path) => path,
        Err(e) => return error_result(e),
    };
    let run_root = provision_module(run_root, &module_path, &tokens[0]);
    let spec = crate::sandbox::oci::BundleSpec {
        run_root: run_root.clone(),
        step_slug: ctx.step.name.clone(),
        command: tokens.to_vec(),
        env: step_env(ctx),
        timeout_hint: ctx.step.timeout_secs,
    };
    // Bundles live next to the run root (never inside it — the run root is
    // the user-visible `/workspace/context` bind).
    let bundle_dir = ctx
        .workflow_root
        .join("bundles")
        .join(&ctx.run_id)
        .join(&ctx.step.name);
    // Copying a provisioned runtime tree can take time; keep node
    // heartbeats and unrelated executions responsive while preparing the
    // private tree.
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
        Ok((0, stdout)) => finish_from_output_json(&run_root.join(&ctx.step.name), stdout),
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

/// Ensure the resolved module lives inside the run tree (the runc guest
/// only sees `/workspace/context`). Library modules are copied in under
/// their relative command path; an existing file is kept so concurrent
/// steps sharing one library module never copy over each other's reads.
fn provision_module(
    run_root: &std::path::Path,
    module_path: &std::path::Path,
    rel_token: &str,
) -> PathBuf {
    if module_path.starts_with(run_root) {
        return run_root.to_path_buf();
    }
    let target = match module_host_path(run_root, rel_token) {
        Ok(path) => path,
        Err(_) => return run_root.to_path_buf(),
    };
    if !target.is_file() {
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::copy(module_path, &target);
    }
    run_root.to_path_buf()
}

/// Success path: parse the step's optional `output.json`.
pub(crate) fn finish_from_output_json(
    step_dir: &std::path::Path,
    output_text: String,
) -> StepResult {
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

pub(crate) fn done_result(output_text: String, output_json: Option<Value>) -> StepResult {
    StepResult {
        outcome: StepOutcome::Done,
        error: None,
        output_text,
        output_json,
        session_id: None,
    }
}

pub(crate) fn error_result(error: String) -> StepResult {
    StepResult {
        outcome: StepOutcome::Error,
        error: Some(error),
        output_text: String::new(),
        output_json: None,
        session_id: None,
    }
}

/// Last `max_bytes` of `text` on a char boundary, marked when truncated.
pub(crate) fn tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    format!("...\n{}", &text[start..])
}
