//! Process-level smoke for the split fleet binaries.
//!
//! Boots one real `opencoder-server` (port 0) plus one real `opencoder-agent`
//! worker against it, then walks the HMAC signature contract over raw TCP:
//! the SPA shell and `/api/time` stay unsigned, every other route demands
//! `x-sig-timestamp` + `x-sig` (401 when missing/stale/tampered, 409 on exact
//! replay), and a registered node shows up in `GET /api/nodes` with a source
//! address and a fresh heartbeat while its process keeps running.
//!
//! Deliberately NOT covered: node task dispatch (needs an LLM) and the legacy
//! process verbs — `server`/`client`/`node` are deleted, and spawning one
//! would not even error (clap would read it as a free-form prompt and launch
//! a live agent), so nothing here ever spawns them.

mod support;

use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opencoder_core::auth_sig;

const TOKEN: &str = "daemon-smoke-token";
const NODE_NAME: &str = "smoke-node-1";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Kill-and-reap on drop, so a failing assert never leaks daemon processes.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start `opencoder-server` on an OS-picked port; return the guard plus the
/// base URL parsed from the `listening on http://` stdout line. The blocking
/// read is fine: the server prints the line promptly after binding.
fn spawn_server(workdir: &std::path::Path) -> (Proc, String) {
    let mut server = Proc(
        Command::new(support::sibling_bin(support::SERVER_BIN))
            .arg("--workdir")
            .arg(workdir)
            .args(["--host", "127.0.0.1", "--port", "0", "--token", TOKEN])
            // Keep the per-workdir SQLite store (data_dir_for →
            // <XDG_DATA_HOME>/opencoder/<digest(workdir)>) inside the test's
            // tempdir instead of polluting the real HOME.
            .env("XDG_DATA_HOME", workdir.join("xdg"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn opencoder-server"),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = String::new();
    loop {
        assert!(Instant::now() < deadline, "opencoder-server did not start");
        let mut buf = [0u8; 1024];
        let count = server
            .0
            .stdout
            .as_mut()
            .unwrap()
            .read(&mut buf)
            .unwrap_or(0);
        output.push_str(&String::from_utf8_lossy(&buf[..count]));
        if let Some(line) = output.lines().find(|l| l.contains("listening on http://")) {
            let base = line
                .split("listening on ")
                .nth(1)
                .unwrap()
                .trim()
                .to_string();
            return (server, base);
        }
    }
}

/// Start `opencoder-agent` pointing at `remote`; stderr lands in `log` so a
/// failure prints the node's own words instead of a bare assert.
///
/// DAG claiming stays ON (no --no-dag): the DAG hook's eager construction
/// (uplink + local store + LLM client from the seeded stub config) succeeds
/// offline because client construction never dials, and the node runner
/// downgrades failed DAG claim polls to warnings — so the default worker
/// wiring is exercised for free while the heartbeat test stays deterministic.
fn spawn_node(workdir: &std::path::Path, remote: &str, log: &std::path::Path) -> Proc {
    let stderr = File::create(log).expect("create node stderr capture file");
    Proc(
        Command::new(support::sibling_bin(support::AGENT_BIN))
            .arg("--workdir")
            .arg(workdir)
            .args([
                "--remote",
                remote,
                "--token",
                TOKEN,
                "--name",
                NODE_NAME,
                "--workflow-root",
            ])
            .arg(workdir.join("workflow"))
            // Same store hygiene as the server spawn: the agent's local
            // store (opened eagerly by the DAG hook) dies with the tempdir.
            .env("XDG_DATA_HOME", workdir.join("xdg"))
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn opencoder-agent"),
    )
}

/// A node resolves its LLM client from config at startup (before the
/// heartbeat loop starts). No task is ever dispatched here, so a dummy
/// loopback provider carries `run_node` past client construction with zero
/// network use and no credentials — deterministic even on machines that have
/// no global ~/.opencoder config.
fn seed_llm_config(workdir: &std::path::Path) {
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder/config.json"),
        r#"{"model":"stub/m1","providers":{"stub":{"base_url":"http://127.0.0.1:9/v1","api_key":"smoke-dummy-key","model":"m1"}}}"#,
    )
    .unwrap();
}

/// Last `keep` lines of a text file, for failure messages.
fn tail(path: &std::path::Path, keep: usize) -> String {
    std::fs::read_to_string(path)
        .map(|text| {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(keep);
            lines[start..].join("\n")
        })
        .unwrap_or_else(|e| format!("<unreadable: {e}>"))
}

/// One raw HTTP exchange over a fresh TCP connection (`connection: close`).
/// `ts` + `sig` = None sends the request unsigned; both must come together.
fn raw_http(
    base: &str,
    method: &str,
    path: &str,
    body: &str,
    ts: Option<i64>,
    sig: Option<&str>,
) -> (u16, String) {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).expect("connect to daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let auth = match (ts, sig) {
        (Some(t), Some(s)) => format!(
            "{ts_header}: {t}\r\n{sig_header}: {s}\r\n",
            ts_header = auth_sig::TS_HEADER,
            sig_header = auth_sig::SIG_HEADER,
        ),
        (None, None) => String::new(),
        _ => panic!("ts and sig must be passed together"),
    };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {host}\r\n{auth}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let response = String::from_utf8(response).unwrap_or_default();
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or(("", ""));
    let status = head
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .parse()
        .unwrap_or(0);
    (status, body.to_string())
}

/// Signed request with a fresh timestamp, so every call carries a brand-new
/// signature and can never trip the server's replay cache.
fn signed(base: &str, method: &str, path: &str, body: &str) -> (u16, String) {
    let ts = now_ms();
    let sig = auth_sig::sign_hex(
        TOKEN,
        &auth_sig::canonical(method, path, ts, body.as_bytes()),
    );
    raw_http(base, method, path, body, Some(ts), Some(&sig))
}

#[test]
fn daemon_server_and_client_end_to_end() {
    let server_dir = tempfile::tempdir().unwrap();
    let node_dir = tempfile::tempdir().unwrap();
    seed_llm_config(node_dir.path());

    let (_server, base) = spawn_server(server_dir.path());

    // Unsigned surfaces: the SPA shell and the clock bootstrap.
    let (status, html) = raw_http(&base, "GET", "/", "", None, None);
    assert_eq!(status, 200, "SPA shell must load without a signature");
    assert!(html.contains("<html"), "GET / must return the shell HTML");
    let (status, time_body) = raw_http(&base, "GET", "/api/time", "", None, None);
    assert_eq!(status, 200, "/api/time is the unsigned clock bootstrap");
    assert!(
        time_body.contains("server_time_ms"),
        "clock bootstrap must expose a millisecond field: {time_body}"
    );

    // Signed /api/health against the failure modes.
    let (status, why) = raw_http(&base, "GET", "/api/health", "", None, None);
    assert_eq!(status, 401, "missing signature must be refused: {why}");

    let ts_ok = now_ms();
    let sig_ok = auth_sig::sign_hex(
        TOKEN,
        &auth_sig::canonical("GET", "/api/health", ts_ok, b""),
    );
    let (status, why) = raw_http(&base, "GET", "/api/health", "", Some(ts_ok), Some(&sig_ok));
    assert_eq!(status, 200, "valid signature must pass: {why}");

    let stale = now_ms() - auth_sig::REPLAY_WINDOW_MS - 60_000;
    let stale_sig = auth_sig::sign_hex(
        TOKEN,
        &auth_sig::canonical("GET", "/api/health", stale, b""),
    );
    let (status, why) = raw_http(
        &base,
        "GET",
        "/api/health",
        "",
        Some(stale),
        Some(&stale_sig),
    );
    assert_eq!(
        status, 401,
        "out-of-window timestamp must be refused: {why}"
    );

    // Sign the empty body but ship a non-empty one: the body hash is part of
    // the canonical string, so the middleware must see a mismatch.
    let ts_now = now_ms();
    let empty_sig = auth_sig::sign_hex(
        TOKEN,
        &auth_sig::canonical("GET", "/api/health", ts_now, b""),
    );
    let (status, why) = raw_http(
        &base,
        "GET",
        "/api/health",
        "{}",
        Some(ts_now),
        Some(&empty_sig),
    );
    assert_eq!(status, 401, "tampered body must be refused: {why}");

    // Exact duplicate of the accepted request above (same ts + sig bytes) is
    // a replay, not a fresh acceptance.
    let (status, why) = raw_http(&base, "GET", "/api/health", "", Some(ts_ok), Some(&sig_ok));
    assert_eq!(status, 409, "exact signature replay must be 409: {why}");

    // Fleet: the worker registers and heartbeats.
    let node_log = node_dir.path().join("node.stderr.log");
    let mut node = spawn_node(node_dir.path(), &base, &node_log);

    let deadline = Instant::now() + Duration::from_secs(20);
    let record = loop {
        let (status, body) = signed(&base, "GET", "/api/nodes", "");
        assert_eq!(status, 200, "signed /api/nodes must answer: {body}");
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        if let Some(n) = json["nodes"]
            .as_array()
            .and_then(|ns| ns.iter().find(|n| n["name"].as_str() == Some(NODE_NAME)))
            .cloned()
        {
            break n;
        }
        assert!(
            Instant::now() < deadline,
            "node {NODE_NAME} never registered within 20s\n--- node stderr ---\n{}",
            tail(&node_log, 30)
        );
        std::thread::sleep(Duration::from_millis(100));
    };

    assert!(
        record["addr"].as_str().is_some_and(|a| !a.is_empty()),
        "node record must carry a non-empty source address: {record}"
    );
    let seen = record["last_seen_at"].as_i64().unwrap_or_default();
    assert!(
        seen > 0 && now_ms() - seen < 60_000,
        "node heartbeat must be recent: {record}"
    );
    assert_ne!(
        record["status"].as_str(),
        Some("lost"),
        "a freshly heartbeating node must not be lost: {record}"
    );

    // Heartbeat loop still running: the worker never exits on its own.
    assert!(
        node.0.try_wait().expect("try_wait node").is_none(),
        "opencoder-agent exited early\n--- node stderr ---\n{}",
        tail(&node_log, 30)
    );
}
