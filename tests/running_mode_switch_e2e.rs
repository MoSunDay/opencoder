//! Real-binary regression: while an actual server drain is blocked in its LLM
//! request, dedicated switch endpoints (POST /agent, POST /handoff, the
//! `agent` field on /prompt) refuse with 409 and persist nothing, while
//! textual mode commands (/plan ...) are admitted and applied by the runner
//! at the idle boundary.
//!
//! P0 note: the server process is now the dedicated `opencoder-server` binary
//! (formerly `opencoder daemon --server`); the "running mode switch" under
//! test here is the plan/act agent-mode switching, which survived the
//! three-binary split unchanged.

mod support;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use opencoder_core::auth_sig;

const TOKEN: &str = "running-mode-e2e-token";

struct BlockingStub {
    port: u16,
    entered: Arc<(Mutex<bool>, Condvar)>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingStub {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let entered = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let entered_thread = entered.clone();
        let release_thread = release.clone();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let entered = entered_thread.clone();
                let release = release_thread.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.read(&mut [0u8; 16 * 1024]);
                    *entered.0.lock().unwrap() = true;
                    entered.1.notify_all();
                    let mut allowed = release.0.lock().unwrap();
                    while !*allowed {
                        allowed = release.1.wait(allowed).unwrap();
                    }
                    let body = br#"{"error":"released blocking stub"}"#;
                    let head = format!(
                        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                });
            }
        });
        Self {
            port,
            entered,
            release,
        }
    }

    fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut entered = self.entered.0.lock().unwrap();
        while !*entered {
            assert!(Instant::now() < deadline, "LLM stub was never called");
            entered = self
                .entered
                .1
                .wait_timeout(entered, Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }

    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}

/// Minimal OpenAI-compatible streaming server for real-binary happy-path
/// coverage. Each connection consumes one scripted reply and records the JSON
/// request body so the test can verify what crossed the actual HTTP boundary.
struct CompletionStub {
    port: u16,
    requests: Arc<(Mutex<Vec<String>>, Condvar)>,
}

impl CompletionStub {
    fn spawn(replies: impl IntoIterator<Item = &'static str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let requests_thread = requests.clone();
        let replies = Arc::new(Mutex::new(replies.into_iter().collect::<VecDeque<_>>()));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let body = read_http_body(&mut stream);
                requests_thread.0.lock().unwrap().push(body);
                requests_thread.1.notify_all();
                let reply = replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected extra LLM request");
                write_completion(&mut stream, reply);
            }
        });
        Self { port, requests }
    }

    fn wait_for_requests(&self, count: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut requests = self.requests.0.lock().unwrap();
        while requests.len() < count {
            assert!(
                Instant::now() < deadline,
                "expected {count} LLM requests, got {}",
                requests.len()
            );
            requests = self
                .requests
                .1
                .wait_timeout(requests, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        requests.clone()
    }
}

fn read_http_body(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = Vec::new();
    let (header_end, content_len) = loop {
        let mut chunk = [0u8; 8192];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "LLM request closed before headers completed");
        request.extend_from_slice(&chunk[..count]);
        let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..end]);
        let content_len = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .expect("LLM request must carry content-length");
        break (end + 4, content_len);
    };
    while request.len() < header_end + content_len {
        let mut chunk = [0u8; 8192];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "LLM request closed before body completed");
        request.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(request[header_end..header_end + content_len].to_vec()).unwrap()
}

fn write_completion(stream: &mut TcpStream, text: &str) {
    let delta = serde_json::json!({"choices": [{"delta": {"content": text}}]});
    let body = format!(
        "data: {delta}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body.as_bytes()).unwrap();
}

struct ServerGuard(std::process::Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(workdir: &std::path::Path) -> (ServerGuard, String) {
    let mut server = ServerGuard(
        Command::new(support::sibling_bin(support::SERVER_BIN))
            .arg("--workdir")
            .arg(workdir)
            .args(["--host", "127.0.0.1", "--port", "0", "--token", TOKEN])
            // Keep the per-workdir SQLite store inside the test's tempdir;
            // the workdir is stable across the restart below, so the digest
            // (and therefore the persisted sessions) survive the respawn.
            .env("XDG_DATA_HOME", workdir.join("xdg"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
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
        if let Some(line) = output
            .lines()
            .find(|line| line.contains("listening on http://"))
        {
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn http(base: &str, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // HMAC signature over THIS request (wire format: crates/core/src/auth_sig.rs).
    // A fresh timestamp per call keeps every signature unique — resending the
    // same ts+sig pair inside the replay window would be a 409, not a 200.
    let ts = now_ms();
    let sig = auth_sig::sign_hex(
        TOKEN,
        &auth_sig::canonical(method, path, ts, body.as_bytes()),
    );
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {host}\r\n{ts_header}: {ts}\r\n{sig_header}: {sig}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len(),
        ts_header = auth_sig::TS_HEADER,
        sig_header = auth_sig::SIG_HEADER,
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let response = String::from_utf8(response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let json = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn wait_for_session(
    base: &str,
    sid: &str,
    label: &str,
    ready: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (status, session) = http(base, "GET", &format!("/api/sessions/{sid}"), "");
        assert_eq!(
            status, 200,
            "failed to read session while waiting for {label}"
        );
        if ready(&session) {
            return session;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn real_server_rejects_running_mode_switches_until_idle() {
    let stub = BlockingStub::spawn();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".opencoder")).unwrap();
    std::fs::write(
        tmp.path().join(".opencoder/config.json"),
        format!(
            r#"{{"model":"stub/m1","providers":{{"stub":{{"base_url":"http://127.0.0.1:{}/v1","api_key":"test-key","model":"m1"}}}}}}"#,
            stub.port
        ),
    )
    .unwrap();
    std::fs::write(tmp.path().join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let (_server, base) = spawn_server(tmp.path());

    let (status, created) = http(&base, "POST", "/api/sessions", r#"{"agent":"act"}"#);
    assert_eq!(status, 200);
    let sid = created["id"].as_str().unwrap();
    assert_eq!(
        http(
            &base,
            "POST",
            &format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"keep running","delivery":"steer"}"#,
        )
        .0,
        200
    );
    stub.wait_until_entered();

    // Dedicated switch endpoints still refuse while a drain runs.
    for (path, body) in [
        (format!("/api/sessions/{sid}/agent"), r#"{"value":"plan"}"#),
        (format!("/api/sessions/{sid}/handoff"), r#"{"extra":"now"}"#),
    ] {
        assert_eq!(http(&base, "POST", &path, body).0, 409, "{path} accepted");
    }
    // Textual mode commands are no longer admission-time mode switches:
    // admitted (200) while running, applied by the runner at the boundary.
    // The dedicated `agent` field on /prompt is still refused.
    assert_eq!(
        http(
            &base,
            "POST",
            &format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"/plan later","delivery":"queue","skill":"reviewer"}"#,
        )
        .0,
        200,
        "queued mode command must be admitted while running"
    );
    assert_eq!(
        http(
            &base,
            "POST",
            &format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"x","delivery":"queue","agent":"plan"}"#,
        )
        .0,
        409,
        "agent field refused while running"
    );

    let (status, session) = http(&base, "GET", &format!("/api/sessions/{sid}"), "");
    assert_eq!(status, 200);
    assert_eq!(
        session["meta"]["agent"], "act",
        "queued switch not applied mid-turn"
    );
    assert_eq!(
        session["meta"]["skill"], "reviewer",
        "skill persisted at admission, consumed at boundary"
    );
    assert!(!session["messages"].to_string().contains("/plan later"));

    stub.release();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let switched = http(
            &base,
            "POST",
            &format!("/api/sessions/{sid}/agent"),
            r#"{"value":"plan"}"#,
        );
        if switched.0 == 200 {
            break;
        }
        assert_eq!(switched.0, 409);
        assert!(
            Instant::now() < deadline,
            "queued /plan later never applied"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let (_, session) = http(&base, "GET", &format!("/api/sessions/{sid}"), "");
    assert_eq!(session["meta"]["agent"], "plan");
}

/// Real process + real HTTP + real OpenAI-SSE client: a plan answer must
/// survive `/act_clear_context` as the sole execution directive, while the
/// discarded planning prompt cannot leak into act or post-restart context.
#[test]
fn real_server_clear_context_executes_preserved_plan_in_act() {
    const SID: &str = "plan-clear-handoff-e2e";
    const PLAN: &str = "EXECUTE_DEPLOYMENT_PLAN_42";
    const RESULT: &str = "ACT_EXECUTION_COMPLETE_42";
    const RESUMED: &str = "RESUMED_ACT_COMPLETE_42";

    let stub = CompletionStub::spawn([PLAN, RESULT, RESUMED]);
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".opencoder")).unwrap();
    std::fs::write(
        tmp.path().join(".opencoder/config.json"),
        format!(
            r#"{{"model":"stub/m1","providers":{{"stub":{{"base_url":"http://127.0.0.1:{}/v1","api_key":"test-key","model":"m1"}}}}}}"#,
            stub.port
        ),
    )
    .unwrap();
    std::fs::write(tmp.path().join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let (server, base) = spawn_server(tmp.path());

    // Prompting an absent id through the production endpoint creates a titled
    // plan session, avoiding the unrelated automatic title-generation call.
    let path = format!("/api/sessions/{SID}/prompt");
    let original_prompt = "draft a rollout with obsolete planning chatter";
    let first = serde_json::json!({
        "prompt": original_prompt,
        "delivery": "queue",
        "agent": "plan"
    })
    .to_string();
    assert_eq!(http(&base, "POST", &path, &first).0, 200);
    wait_for_session(&base, SID, "plan reply", |session| {
        session["draining"] == false && session["messages"].to_string().contains(PLAN)
    });

    let clear = r#"{"prompt":"/act_clear_context","delivery":"queue"}"#;
    assert_eq!(http(&base, "POST", &path, clear).0, 200);
    let session = wait_for_session(&base, SID, "act execution", |session| {
        session["draining"] == false
            && session["meta"]["agent"] == "act"
            && session["messages"].to_string().contains(RESULT)
    });

    let requests = stub.wait_for_requests(2);
    assert_eq!(requests.len(), 2, "handoff must make exactly one act call");
    let act_request: serde_json::Value = serde_json::from_str(&requests[1]).unwrap();
    let act_wire = act_request["messages"].to_string();
    assert!(
        act_wire.contains(PLAN),
        "preserved plan missing from act request: {act_wire}"
    );
    assert!(
        !act_wire.contains(original_prompt),
        "cleared planning chatter leaked into act request: {act_wire}"
    );

    let stored_history = session["messages"].to_string();
    assert!(stored_history.contains(PLAN) && stored_history.contains(RESULT));
    // Clear-context is a resume boundary, not destructive history deletion.
    // Restart the actual opencoder-server and prove the boundary—not row removal—keeps
    // pre-clear planning chatter out of the next model request.
    assert!(
        stored_history.contains(original_prompt),
        "the boundary must not destructively delete history: {stored_history}"
    );
    assert!(
        session["meta"]["handoff_seq"].is_number(),
        "resume boundary was not persisted: {}",
        session["meta"]
    );

    drop(server);
    let (_resumed_server, resumed_base) = spawn_server(tmp.path());
    let resume_prompt = "verify execution after daemon restart";
    let resumed_body = serde_json::json!({
        "prompt": resume_prompt,
        "delivery": "queue"
    })
    .to_string();
    assert_eq!(http(&resumed_base, "POST", &path, &resumed_body).0, 200);
    let resumed = wait_for_session(&resumed_base, SID, "resumed act reply", |session| {
        session["draining"] == false
            && session["meta"]["agent"] == "act"
            && session["messages"].to_string().contains(RESUMED)
    });

    let requests = stub.wait_for_requests(3);
    let resumed_request: serde_json::Value = serde_json::from_str(&requests[2]).unwrap();
    let resumed_wire = resumed_request["messages"].to_string();
    assert!(resumed_wire.contains(PLAN) && resumed_wire.contains(RESULT));
    assert!(resumed_wire.contains(resume_prompt));
    assert!(
        !resumed_wire.contains(original_prompt),
        "resume boundary leaked cleared chatter after restart: {resumed_wire}"
    );
    assert_eq!(resumed["meta"]["agent"], "act");
}
