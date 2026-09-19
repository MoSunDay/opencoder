//! `opencoder` host import bindings (wasm32 only) plus the shared wasm
//! step flow used by the three bins.
//!
//! Wire signatures (must mirror dag-runtime `host_imports`):
//! `opencoder_run_op(op_id_ptr, op_id_len, args_ptr, args_len) -> i32`
//! (child exit code) and `opencoder_http_probe(url_ptr, url_len, expect,
//! timeout_ms, retries) -> i32` (HTTP status on success, negative error
//! code otherwise). Both trap the guest on host-side failure, so the
//! wrappers never see an "unknown" result — a step either gets numbers
//! or dies, which is the fail-closed contract.

#[cfg(target_arch = "wasm32")]
mod wire {
    #[link(wasm_import_module = "opencoder")]
    extern "C" {
        #[link_name = "opencoder_run_op"]
        fn sys_run_op(
            op_id_ptr: *const u8,
            op_id_len: i32,
            args_ptr: *const u8,
            args_len: i32,
        ) -> i32;
        #[link_name = "opencoder_http_probe"]
        fn sys_http_probe(
            url_ptr: *const u8,
            url_len: i32,
            expect: i32,
            timeout_ms: i32,
            retries: i32,
        ) -> i32;
    }

    /// Run one whitelisted op. `args` is one string; the node splits it
    /// on spaces into argv (argv-only, no shell).
    pub fn run_op(op_id: &str, args: &str) -> i32 {
        unsafe {
            sys_run_op(
                op_id.as_ptr(),
                op_id.len() as i32,
                args.as_ptr(),
                args.len() as i32,
            )
        }
    }

    /// Probe `url` until it answers 2xx. `timeout_ms` bounds one attempt,
    /// `retries` adds extra attempts with a runtime-chosen pause.
    pub fn http_probe(url: &str, timeout_ms: i32, retries: i32) -> i32 {
        unsafe { sys_http_probe(url.as_ptr(), url.len() as i32, 0, timeout_ms, retries) }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wire::{http_probe, run_op};

/// Read one op's evidence log from the step dir (the runtime preopens
/// `/workspace/context`, so plain `std::fs` works inside the guest).
pub fn read_evidence(step_dir: &std::path::Path, op_id: &str) -> Result<String, String> {
    let path = step_dir.join(super::evidence_rel_path(op_id));
    std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// The shared deploy-step flow: run the deploy op, read back its
/// evidence, discover the endpoint from the tail JSON, probe the ready
/// path, and write `output.json`. Non-zero op exit, a missing tail JSON
/// or a failed probe all produce a `down` verdict written to
/// `output.json` and a NON-ZERO process exit — the DAG runtime marks the
/// step Error and downstream steps never see `ok` context.
#[cfg(target_arch = "wasm32")]
pub fn deploy_and_probe(step_dir: &std::path::Path, op_id: &str, ready_path: &str) -> ! {
    let exit = run_op(op_id, "");
    let evidence = read_evidence(step_dir, op_id).unwrap_or_else(|e| {
        eprintln!("env step: {e}");
        std::process::exit(1);
    });
    let parsed = super::parse_op_log(&evidence);
    let tail = super::tail_json(&parsed.output);
    let endpoint = tail.as_ref().and_then(super::endpoint_of);
    let (endpoint, probe) = match endpoint {
        Some(ep) => {
            let url = format!("{ep}{ready_path}");
            (ep, http_probe(&url, 2_000, 30))
        }
        None => (String::new(), super::PROBE_ERR_URL),
    };
    let out = super::deploy_output(
        parsed.exit.or(Some(exit)),
        parsed.killed.as_deref(),
        &endpoint,
        probe,
        op_id,
    );
    if let Err(e) = super::write_output_json(step_dir, &out) {
        eprintln!("env step: {e}");
        std::process::exit(1);
    }
    let up = out["status"] == "up";
    if !up {
        eprintln!("env step: environment did not come up (http={probe})");
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// The shared harness-step flow: run the harness op, parse the NDJSON
/// stages out of its evidence, aggregate, and write `output.json`. Any
/// failed stage (or op-level failure) exits non-zero.
#[cfg(target_arch = "wasm32")]
pub fn run_harness(step_dir: &std::path::Path, op_id: &str) -> ! {
    let exit = run_op(op_id, "");
    let evidence = read_evidence(step_dir, op_id).unwrap_or_else(|e| {
        eprintln!("harness: {e}");
        std::process::exit(1);
    });
    let parsed = super::parse_op_log(&evidence);
    let stages = super::parse_stages(&parsed.output);
    let out = super::harness_output(
        parsed.exit.or(Some(exit)),
        parsed.killed.as_deref(),
        &stages,
        op_id,
    );
    if let Err(e) = super::write_output_json(step_dir, &out) {
        eprintln!("harness: {e}");
        std::process::exit(1);
    }
    if out["status"] != "passed" {
        eprintln!("harness: failed stages {:?}", super::failed_stages(&stages));
        std::process::exit(1);
    }
    std::process::exit(0);
}
