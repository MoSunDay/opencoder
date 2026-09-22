//! Native unit tests for the op registry + probe host imports (no wasm
//! involved): whitelist, env passthrough, evidence file, kill semantics
//! and the probe state machine over a real loopback listener.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use opencoder_core::config::DagOpConfig;

use super::*;

/// Unique-per-test secret so parallel tests never race on env vars.
fn unique_secret(tag: &str) -> String {
    format!("OPENCODER_TEST_SECRET_{}_{}", tag, rand_thread_key())
}

/// Deterministic-enough discriminator without a rand dependency.
fn rand_thread_key() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
}

/// Write an executable `/bin/sh` script into the tempdir.
fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    make_executable(&path);
    path
}

fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

fn state(dir: &std::path::Path, ops: BTreeMap<String, DagOpConfig>) -> HostState {
    HostState {
        ops,
        run_root: dir.to_path_buf(),
        step_dir: dir.join("eob-up"),
        cancel: Arc::new(AtomicBool::new(false)),
        output: None,
    }
}

fn op(command: &str, env_keys: &[&str], timeout_secs: Option<u64>) -> DagOpConfig {
    DagOpConfig {
        command: command.to_string(),
        env_keys: env_keys.iter().map(|k| k.to_string()).collect(),
        timeout_secs,
        termination_grace_secs: 0,
    }
}

fn ops(entries: &[(&str, DagOpConfig)]) -> BTreeMap<String, DagOpConfig> {
    entries
        .iter()
        .map(|(id, cfg)| ((*id).to_string(), cfg.clone()))
        .collect()
}

fn op_log(state: &HostState, op_id: &str) -> String {
    std::fs::read_to_string(state.step_dir.join("ops").join(format!("{op_id}.log")))
        .unwrap_or_else(|e| panic!("op log for {op_id}: {e}"))
}

#[test]
fn registered_op_runs_argv_only_with_declared_env_and_persists_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let secret_key = unique_secret("ok");
    std::env::set_var(&secret_key, "sec-1234");
    let script = script(
        tmp.path(),
        "noop.sh",
        r#"echo "argv=[$1] [$2]"
echo "secret=${secret_key}"
echo "home=${HOME:-unset}"
echo "last-json {\"ok\": true}""#
            .replace("secret_key", &secret_key)
            .as_str(),
    );
    let mut st = state(
        tmp.path(),
        ops(&[(
            "eob_deploy",
            op(script.to_str().unwrap(), &[&secret_key], None),
        )]),
    );
    let code = run_op(&mut st, "eob_deploy", "http://127.0.0.1:30384 extra")
        .expect("registered op must run");
    assert_eq!(code, 0);
    let log = op_log(&st, "eob_deploy");
    assert!(log.contains("# op eob_deploy exit=0"), "log head: {log}");
    assert!(
        log.contains("argv=[http://127.0.0.1:30384] [extra]"),
        "args appended: {log}"
    );
    assert!(
        log.contains("secret=sec-1234"),
        "declared env passed: {log}"
    );
    assert!(log.contains("home=unset"), "undeclared env dropped: {log}");
    std::env::remove_var(&secret_key);
}

#[test]
fn unknown_or_malformed_op_ids_fail_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let mut st = state(tmp.path(), ops(&[("known", op("/bin/true", &[], None))]));
    let err = run_op(&mut st, "ghost", "").unwrap_err().to_string();
    assert!(err.contains("unknown op `ghost`"), "{err}");
    assert!(err.contains("dag.ops"), "{err}");
    // A traversal-shaped id is rejected before touching the filesystem.
    let err = run_op(&mut st, "../evil", "").unwrap_err().to_string();
    assert!(err.contains("malformed op id"), "{err}");
    // An empty command registration is a config error, not a shell.
    let mut st2 = state(tmp.path(), ops(&[("blank", op("   ", &[], None))]));
    let err = run_op(&mut st2, "blank", "").unwrap_err().to_string();
    assert!(err.contains("empty command"), "{err}");
}

#[test]
fn nonzero_exit_code_is_returned_not_errored() {
    let tmp = tempfile::tempdir().unwrap();
    let script = script(tmp.path(), "fail.sh", "echo boom >&2; exit 3");
    let mut st = state(
        tmp.path(),
        ops(&[("probe_op", op(script.to_str().unwrap(), &[], None))]),
    );
    assert_eq!(run_op(&mut st, "probe_op", "").unwrap(), 3);
    let log = op_log(&st, "probe_op");
    assert!(log.contains("exit=3"), "{log}");
    assert!(log.contains("boom"), "stderr captured: {log}");
}

#[test]
fn op_timeout_kills_the_child_and_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let script = script(tmp.path(), "sleep.sh", "sleep 30");
    let mut st = state(
        tmp.path(),
        ops(&[("slow", op(script.to_str().unwrap(), &[], Some(1)))]),
    );
    let started = std::time::Instant::now();
    let err = run_op(&mut st, "slow", "").unwrap_err().to_string();
    assert!(err.contains("timeout after 1s"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "kill must be prompt"
    );
    // Partial evidence still lands for the killed op.
    assert!(op_log(&st, "slow").contains("killed"), "log missing");
}

#[test]
fn op_output_overflow_kills_the_child_and_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let script = script(
        tmp.path(),
        "spew.sh",
        "yes spam-spam-spam | head -c 1048576",
    );
    let mut st = state(
        tmp.path(),
        ops(&[("spew", op(script.to_str().unwrap(), &[], None))]),
    );
    let err = run_op(&mut st, "spew", "").unwrap_err().to_string();
    assert!(err.contains("exceeds"), "{err}");
    let log = op_log(&st, "spew");
    assert!(log.contains("truncated=true"), "{log}");
    assert!(
        log.len() < 300 * 1024,
        "persisted log must stay near the cap: {}",
        log.len()
    );
}

#[test]
fn op_cancellation_flag_kills_the_child() {
    let tmp = tempfile::tempdir().unwrap();
    let script = script(tmp.path(), "hang.sh", "sleep 30");
    let mut st = state(
        tmp.path(),
        ops(&[("hang", op(script.to_str().unwrap(), &[], None))]),
    );
    st.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let err = run_op(&mut st, "hang", "").unwrap_err().to_string();
    assert!(err.contains("cancelled"), "{err}");
}

#[test]
fn op_cancellation_grace_allows_durable_cleanup_receipt() {
    let tmp = tempfile::tempdir().unwrap();
    let script = script(tmp.path(), "cleanup.sh", "trap 'echo restored > cleanup.txt; exit 0' TERM\nsleep 3 &\necho ready > ready.txt\nwhile :; do sleep 0.05; done");
    let mut cfg = op(script.to_str().unwrap(), &[], None);
    cfg.termination_grace_secs = 2;
    let mut st = state(tmp.path(), ops(&[("cleanup", cfg)]));
    let flag = st.cancel.clone();
    let root = st.run_root.clone();
    let trigger = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !root.join("ready.txt").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let started = Instant::now();
    let err = run_op(&mut st, "cleanup", "").unwrap_err().to_string();
    assert!(started.elapsed() < Duration::from_secs(2), "descendants retained output pipes after controller exit");
    trigger.join().unwrap();
    assert!(err.contains("cancelled"), "{err}");
    assert_eq!(
        std::fs::read_to_string(st.run_root.join("cleanup.txt"))
            .unwrap()
            .trim(),
        "restored"
    );
}

// ---------------------------------------------------------------------------
// probe
// ---------------------------------------------------------------------------

/// Loopback HTTP server answering `status` for every request until the
/// returned guard drops; sends the bound port over the channel.
struct ProbeServer {
    port: u16,
    _tx: mpsc::Sender<()>,
}

impl ProbeServer {
    fn spawn(status: &'static str, body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if rx.try_recv().is_ok() {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        ProbeServer { port, _tx: tx }
    }
}

#[test]
fn probe_accepts_matching_status_and_2xx_default() {
    let server = ProbeServer::spawn("200 OK", "ready");
    let url = format!("http://127.0.0.1:{}/openrusty/ready", server.port);
    assert_eq!(http_probe(&url, 200, 500, 1, &AtomicBool::new(false)), 200);
    // expect <= 0 accepts any 2xx.
    assert_eq!(http_probe(&url, 0, 500, 1, &AtomicBool::new(false)), 200);
    // Path-less URL defaults to "/".
    assert_eq!(
        http_probe(
            &format!("http://127.0.0.1:{}", server.port),
            200,
            500,
            1,
            &AtomicBool::new(false)
        ),
        200
    );
}

#[test]
fn probe_mismatch_and_unreachable_and_bad_url_codes() {
    let server = ProbeServer::spawn("503 Service Unavailable", "nope");
    let url = format!("http://127.0.0.1:{}/ready", server.port);
    assert_eq!(
        http_probe(&url, 200, 200, 1, &AtomicBool::new(false)),
        PROBE_ERR_STATUS,
        "responses arrived but never matched"
    );
    // Nothing listens on an ad-hoc loopback port.
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_url = format!("http://127.0.0.1:{}/x", dead.local_addr().unwrap().port());
    drop(dead);
    assert_eq!(
        http_probe(&dead_url, 200, 200, 2, &AtomicBool::new(false)),
        PROBE_ERR_UNREACHABLE
    );
    // Non-http schemes are URL errors (no TLS fallback, ever).
    assert_eq!(
        http_probe(
            "https://example.com/ready",
            200,
            100,
            0,
            &AtomicBool::new(false)
        ),
        PROBE_ERR_URL
    );
    assert_eq!(
        http_probe("not a url", 200, 100, 0, &AtomicBool::new(false)),
        PROBE_ERR_URL
    );
}

#[test]
fn probe_retries_until_the_window_answers() {
    // Server that fails the first attempt (connection reset) then serves.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        drop(stream); // first attempt dies mid-conversation
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
    });
    let url = format!("http://127.0.0.1:{}/ready", port);
    assert_eq!(http_probe(&url, 200, 500, 3, &AtomicBool::new(false)), 200);
}

#[test]
fn http_url_parser_rejects_garbage_and_maps_ports() {
    assert!(parse_http_url("http://127.0.0.1:30384/openrusty/ready").is_some());
    assert!(parse_http_url("http://localhost").is_some());
    assert!(parse_http_url("ftp://x/y").is_none());
    assert!(parse_http_url("http://host:notaport/").is_none());
    let target = parse_http_url("http://127.0.0.1:9847/healthz").unwrap();
    assert_eq!(target.path, "/healthz");
    assert_eq!(target.host_header, "127.0.0.1:9847");
    assert_eq!(target.addr.port(), 9847);
}
