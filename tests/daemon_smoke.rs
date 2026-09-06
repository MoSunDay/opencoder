//! Process-level smoke for the split fleet binaries.
//!
//! Boots one real `opencoder-server` (port 0) plus one real `opencoder-agent`
//! worker against it, then walks the Bearer-token contract over raw TCP:
//! the SPA shell and `/api/time` stay unsigned, every other route demands
//! `Authorization: Bearer <token>` (401 when missing/wrong), and a registered
//! node shows up in `GET /api/nodes` with a source
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
/// `authorization = None` sends the request without credentials.
fn raw_http(
    base: &str,
    method: &str,
    path: &str,
    body: &str,
    authorization: Option<&str>,
) -> (u16, String) {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).expect("connect to daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let auth = authorization
        .map(|value| format!("authorization: {value}\r\n"))
        .unwrap_or_default();
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

fn authed(base: &str, method: &str, path: &str, body: &str) -> (u16, String) {
    raw_http(base, method, path, body, Some(&format!("Bearer {TOKEN}")))
}

#[test]
fn daemon_server_and_client_end_to_end() {
    let server_dir = tempfile::tempdir().unwrap();
    let node_dir = tempfile::tempdir().unwrap();
    seed_llm_config(node_dir.path());

    let (_server, base) = spawn_server(server_dir.path());

    // Unauthenticated surfaces: the SPA shell and compatibility clock.
    let (status, html) = raw_http(&base, "GET", "/", "", None);
    assert_eq!(status, 200, "SPA shell must load without credentials");
    assert!(html.contains("<html"), "GET / must return the shell HTML");
    let (status, time_body) = raw_http(&base, "GET", "/api/time", "", None);
    assert_eq!(
        status, 200,
        "/api/time remains readable without credentials"
    );
    assert!(
        time_body.contains("server_time_ms"),
        "time endpoint must expose a millisecond field: {time_body}"
    );

    // Protected /api/health accepts only the configured Bearer token.
    let (status, why) = raw_http(&base, "GET", "/api/health", "", None);
    assert_eq!(status, 401, "missing token must be refused: {why}");
    let (status, why) = authed(&base, "GET", "/api/health", "");
    assert_eq!(status, 200, "valid Bearer token must pass: {why}");
    let (status, why) = raw_http(&base, "GET", "/api/health", "", Some("Bearer wrong-token"));
    assert_eq!(status, 401, "wrong Bearer token must be refused: {why}");
    let (status, why) = raw_http(
        &base,
        "GET",
        "/api/health",
        "",
        Some(&format!("Basic {TOKEN}")),
    );
    assert_eq!(status, 401, "non-Bearer auth must be refused: {why}");

    // Fleet: the worker registers and heartbeats.
    let node_log = node_dir.path().join("node.stderr.log");
    let mut node = spawn_node(node_dir.path(), &base, &node_log);

    let deadline = Instant::now() + Duration::from_secs(20);
    let record = loop {
        let (status, body) = authed(&base, "GET", "/api/nodes", "");
        assert_eq!(status, 200, "authenticated /api/nodes must answer: {body}");
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
        record["maintenance_agent_id"]
            .as_str()
            .is_some_and(|a| !a.is_empty())
            && record["snapshot"]["cpu_capacity"]
                .as_f64()
                .is_some_and(|cpu| cpu > 0.0),
        "node registration must expose a maintainer and CPU capacity: {record}"
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
