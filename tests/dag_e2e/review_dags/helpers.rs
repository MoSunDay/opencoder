//! Shared fixtures for the review-dag e2e suite (R1-R5): LLM-free probe
//! server, op scripts, the data-segment WAT builder for host-import modules,
//! publish/dispatch helpers and artifact readers.

use crate::support::fleet_proc::Fleet;
use serde_json::{json, Value};
use std::net::TcpListener;
// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Read a node artifact as JSON.
pub(super) fn read_json(path: &std::path::Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// `<node-data>/dag/<run>/<step>/output.json`.
pub(super) fn step_json(fleet: &Fleet, run: &str, step: &str) -> Value {
    read_json(
        &fleet
            .node_data
            .join("dag")
            .join(run)
            .join(step)
            .join("output.json"),
    )
}

/// `<node-data>/dag/<run>/<step>/ops/<op>.log` (text).
pub(super) fn op_log(fleet: &Fleet, run: &str, step: &str, op: &str) -> String {
    let path = fleet
        .node_data
        .join("dag")
        .join(run)
        .join(step)
        .join("ops")
        .join(format!("{op}.log"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Loopback HTTP 200 responder for `opencoder_http_probe` (every path
/// answers 200 `ok`). The thread parks on `accept`; it dies with the
/// test process like every other e2e stub thread.
pub(super) fn probe_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe server");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf); // request head
            let _ = std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
            );
        }
    });
    port
}

/// Write + chmod 755 a fixture op script.
pub(super) fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write op script");
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path.to_str().unwrap().to_string()
}

/// Wat escape for data segment literals.
pub(super) fn esc(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A step module speaking the review-module contract: run one
/// `dag.ops` op, optionally probe a URL, then write `<step>/output.json`
/// (`ok_json` when the op exited 0 AND the probe — when used — returned
/// HTTP 200; `fail_json` otherwise, and then trap so the step errors,
/// exactly like the real modules exit non-zero on a `down` verdict).
pub(super) fn op_step_wat(
    op_id: &str,
    args: &str,
    probe_url: Option<&str>,
    step: &str,
    ok_json: &str,
    fail_json: &str,
) -> String {
    let url = probe_url.unwrap_or("");
    let probe_call = match probe_url {
        Some(_) => format!(
            "(local.set $probe (call $probe (i32.const 256) (i32.const {}) \
             (i32.const 200) (i32.const 500) (i32.const 2)))",
            url.len()
        ),
        None => String::new(),
    };
    format!(
        r#"(module
  (import "opencoder" "opencoder_run_op"
    (func $run_op (param i32 i32 i32 i32) (result i32)))
  (import "opencoder" "opencoder_http_probe"
    (func $probe (param i32 i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_close"
    (func $fd_close (param i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 64) "{op_id}")
  (data (i32.const 192) "{args}")
  (data (i32.const 256) "{url}")
  (data (i32.const 384) "{out_path}")
  (data (i32.const 512) "{ok}")
  (data (i32.const 896) "{fail}")
  (func (export "_start")
    (local $op i32) (local $probe i32) (local $ok i32)
    (i32.store (i32.const 0) (i32.const 0))
    (local.set $op (call $run_op
      (i32.const 64) (i32.const {op_len}) (i32.const 192) (i32.const {args_len})))
    (local.set $probe (i32.const 200))
    {probe_call}
    (local.set $ok (i32.and
      (i32.eq (local.get $op) (i32.const 0))
      (i32.eq (local.get $probe) (i32.const 200))))
    (if (local.get $ok)
      (then (i32.store (i32.const 16) (i32.const 512)) (i32.store (i32.const 20) (i32.const {ok_len})))
      (else (i32.store (i32.const 16) (i32.const 896)) (i32.store (i32.const 20) (i32.const {fail_len}))))
    (drop (call $path_open (i32.const 3) (i32.const 0) (i32.const 384) (i32.const {path_len})
      (i32.const 9) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 0)))
    (drop (call $fd_write (i32.load (i32.const 0)) (i32.const 16) (i32.const 1) (i32.const 24)))
    (drop (call $fd_close (i32.load (i32.const 0))))
    (if (i32.eq (local.get $ok) (i32.const 0)) (then unreachable))))"#,
        op_id = esc(op_id),
        args = esc(args),
        url = esc(url),
        out_path = esc(&format!("{step}/output.json")),
        ok = esc(ok_json),
        fail = esc(fail_json),
        op_len = op_id.len(),
        args_len = args.len(),
        path_len = format!("{step}/output.json").len(),
        ok_len = ok_json.len(),
        fail_len = fail_json.len(),
    )
}

/// Post a one-wasm-step def and dispatch `run`.
pub(super) fn dispatch_wasm(
    fleet: &Fleet,
    def: &str,
    command: &str,
    run: &str,
    step: &str,
) -> Value {
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec": {"name": def, "steps": [
            {"name": step, "kind": {"type": "wasm", "command": command}},
        ]}}),
    );
    assert_eq!(status, 200, "save def {def}: {body}");
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{def}/dispatch"),
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch {run}: {body}");
    fleet.wait_terminal(run)
}

/// Dispatch a def by NAME (the seeded specs) and wait for the terminal doc.
pub(super) fn dispatch_seeded(fleet: &Fleet, def: &str, run: &str) -> Value {
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{def}/dispatch"),
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch {run}: {body}");
    fleet.wait_terminal(run)
}
