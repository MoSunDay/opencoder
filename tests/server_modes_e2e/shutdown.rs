//! SIGTERM graceful retirement against the real fleet: an operator
//! execution is held mid-round inside the LLM stub (its SSE stream polls
//! forever), a raw spectator SSE connection is open on that execution, and
//! a SIGTERM to the server process must (a) flush a final
//! `event: reconnect` frame to the open stream before closing it and
//! (b) shut the server down cleanly.
//!
//! Exit-status note: the [`Fleet`] guard owns the server child and std
//! offers no adopt/waitid, so "clean exit" is pinned by three combined
//! observations — the `Server HTTP retirement complete` log line (the last
//! statement before `serve_release` returns `Ok`, which is exit code 0),
//! the `/proc` entry leaving the running state, and the reconnect frame
//! (only the graceful retirement path emits one).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::llm_stub::{LlmStub, Script};

const SESSION: &str = "operator-e2e-sigterm-1";
const NODE: &str = "sigterm-node";

#[test]
fn sigterm_drains_open_sse_and_exits_cleanly() {
    let stub = LlmStub::spawn(vec![Script::Hold]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), NODE);

    // One call creates the session and its operator execution; the Hold
    // entry parks the round, so the execution stays running forever.
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({
            "id": SESSION,
            "node_id": fleet.node_id(),
            "agent": "act",
            "prompt": "hold the round",
        }),
    );
    assert_eq!(status, 200, "create operator session: {body}");
    stub.wait_until_entered();

    // Raw spectator SSE on the live execution: prove the stream is live.
    let mut stream = open_sse(&fleet.base, &format!("/api/executions/{SESSION}/events"));
    let mut seen = Vec::new();
    read_until(&mut stream, &mut seen, 5, |head| {
        text(head).contains("text/event-stream")
    });

    let pid = fleet.server_pid();
    let signalled = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(signalled.success(), "kill -TERM {pid} failed");

    // (a) the open stream gets a reconnect frame, then the server closes it.
    read_until(&mut stream, &mut seen, 30, |seen| {
        text(seen).contains("event: reconnect")
    });
    read_until(&mut stream, &mut seen, 30, |_| false); // returns only on EOF
    assert!(
        text(&seen).contains("event: reconnect"),
        "no reconnect frame before EOF, stream: {}",
        text(&seen)
    );

    // (b) the server leaves the running state and logs retirement complete.
    let deadline = Instant::now() + Duration::from_secs(30);
    while process_running(pid) {
        assert!(
            Instant::now() < deadline,
            "server still running 30s after SIGTERM\n--- fleet log tail ---\n{}",
            fleet.log.tail(60)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        fleet.log.tail(400).contains("Server HTTP retirement complete"),
        "graceful retirement never finished\n--- fleet log tail ---\n{}",
        fleet.log.tail(60)
    );
}

/// Lossy text view of the accumulated stream bytes (assertions only).
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Open one Bearer-authenticated SSE connection over raw TCP.
fn open_sse(base: &str, path: &str) -> TcpStream {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).expect("connect for SSE");
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nhost: {host}\r\nauthorization: Bearer {TOKEN}\r\n\
         accept: text/event-stream\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("send SSE GET");
    stream
}

/// Append chunks into `seen` until `done` accepts it, EOF, or `secs`.
/// Reaching the deadline panics with the bytes read so far.
fn read_until(stream: &mut TcpStream, seen: &mut Vec<u8>, secs: u64, done: impl Fn(&[u8]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if done(seen) {
            return;
        }
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return, // EOF: the server closed the stream
            Ok(count) => seen.extend_from_slice(&chunk[..count]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => panic!("SSE read failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "SSE condition not met within {secs}s, stream so far: {}",
            text(seen)
        );
    }
}

/// Whether `pid` is still running (not a zombie, not reaped). The Fleet
/// guard owns reaping, so an exited server shows up as a zombie
/// `/proc/<pid>/stat` entry (state `Z`) or no entry at all.
fn process_running(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
        Err(_) => false,
    }
}
