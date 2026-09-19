//! Host imports for `in_process` wasm steps: the controlled op registry
//! (`opencoder_run_op`) and the HTTP liveness probe
//! (`opencoder_http_probe`).
//!
//! WASI preview 1 has no network and no subprocesses, so a deterministic
//! orchestration module needs the NODE to perform the heavy actions
//! (deploys, session bring-up, harness runs). The capability surface is
//! deliberately tiny and fail-closed:
//!
//! - an op id MUST be registered in `dag.ops` (config-explicit
//!   whitelist); anything else traps the guest and errors the step;
//! - the child runs argv-only (NO shell) with ONLY the `env_keys`-declared
//!   node env vars — tokens never travel through the spec or the guest;
//! - captured op output is bounded ([`OP_LOG_LIMIT_BYTES`]) and persisted
//!   as run evidence at `<step_dir>/ops/<op_id>.log` (guest-visible under
//!   the `/workspace/context` preopen) so the module can read it back;
//! - unknown op / spawn failure / timeout / output overflow all ERROR the
//!   step (fail-closed). A plain non-zero exit code is returned to the
//!   guest, which decides the fail-closed semantics.
//!
//! `sandbox: runc` does NOT link these imports: a module importing them
//! fails to instantiate there instead of silently losing capabilities.
//!
//! Wire signatures (import module `opencoder`, linear memory, ptr/len
//! pairs, guest-side strings must stay alive across the call):
//!
//! ```text
//! opencoder_run_op(op_id_ptr: i32, op_id_len: i32,
//!                  args_ptr: i32, args_len: i32) -> i32   // exit code (>=0)
//! opencoder_http_probe(url_ptr: i32, url_len: i32, expect: i32,
//!                       timeout_ms: i32, retries: i32) -> i32
//! ```
//!
//! Probe return values: the accepted HTTP status code on success, else
//! [`PROBE_ERR_URL`] / [`PROBE_ERR_UNREACHABLE`] / [`PROBE_ERR_STATUS`] /
//! [`PROBE_ERR_CANCELLED`]. `expect <= 0` accepts any 2xx.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use opencoder_core::config::DagOpConfig;
use wasmtime::{Caller, Extern, Linker};

use crate::step_log::StepOutputLog;

mod ops;
mod probe;
#[cfg(test)]
mod tests;

use ops::run_op;
use probe::http_probe;
#[cfg(test)]
use probe::{parse_http_url, PROBE_ERR_STATUS, PROBE_ERR_UNREACHABLE, PROBE_ERR_URL};

/// WASI ctx + host state: the wasmtime store data for in-process steps.
pub(crate) type HostStore = (wasmtime_wasi::p1::WasiP1Ctx, HostState);

/// Per-step host capability state, built by the in-process executor.
pub(crate) struct HostState {
    /// The node's `dag.ops` registry (config-explicit whitelist).
    pub ops: BTreeMap<String, DagOpConfig>,
    /// Host path of the run root — the op child's working directory.
    pub run_root: PathBuf,
    /// Host path of this step's artifact dir; op logs land in `ops/`.
    pub step_dir: PathBuf,
    /// Step cancellation flag: op children are killed promptly.
    pub cancel: Arc<AtomicBool>,
    /// Optional live mirror of op output into the run-scoped step log.
    pub output: Option<StepOutputLog>,
}

/// Collapse an internal anyhow error into a wasmtime trap error (the
/// `anyhow` interop feature is not enabled, so carry the formatted chain).
fn trap(e: anyhow::Error) -> wasmtime::Error {
    wasmtime::Error::msg(format!("{e:#}"))
}

/// Link the `opencoder` import module into an in-process linker. Both
/// imports are sync host calls on the guest's blocking worker.
pub(crate) fn register(linker: &mut Linker<HostStore>) -> std::result::Result<(), wasmtime::Error> {
    linker.func_wrap(
        "opencoder",
        "opencoder_run_op",
        |mut caller: Caller<'_, HostStore>,
         op_ptr: i32,
         op_len: i32,
         args_ptr: i32,
         args_len: i32|
         -> std::result::Result<i32, wasmtime::Error> {
            let op_id = read_string(&mut caller, op_ptr, op_len).map_err(trap)?;
            let args = read_string(&mut caller, args_ptr, args_len).map_err(trap)?;
            run_op(&mut caller.data_mut().1, &op_id, &args).map_err(trap)
        },
    )?;
    linker.func_wrap(
        "opencoder",
        "opencoder_http_probe",
        |mut caller: Caller<'_, HostStore>,
         url_ptr: i32,
         url_len: i32,
         expect: i32,
         timeout_ms: i32,
         retries: i32|
         -> std::result::Result<i32, wasmtime::Error> {
            let url = read_string(&mut caller, url_ptr, url_len).map_err(trap)?;
            let cancel = caller.data().1.cancel.clone();
            Ok(http_probe(&url, expect, timeout_ms, retries, &cancel))
        },
    )?;
    Ok(())
}

/// Read one ptr/len string out of the guest's exported linear memory.
fn read_string(caller: &mut Caller<'_, HostStore>, ptr: i32, len: i32) -> Result<String> {
    if ptr < 0 || len < 0 {
        return Err(anyhow!(
            "host import: negative pointer/length (ptr {ptr}, len {len})"
        ));
    }
    let memory = caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| anyhow!("host import: module does not export `memory`"))?;
    let mut bytes = vec![0u8; len as usize];
    memory
        .read(&mut *caller, ptr as usize, &mut bytes)
        .map_err(|e| anyhow!("host import: read guest memory at {ptr}: {e}"))?;
    String::from_utf8(bytes).map_err(|e| anyhow!("host import: non-UTF-8 argument: {e}"))
}
