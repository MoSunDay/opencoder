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

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use support::llm_stub::{LlmStub, Script};

const TOKEN: &str = "running-mode-e2e-token";

struct ServerGuard(std::process::Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct FleetGuard {
    _server: ServerGuard,
    _agent: ServerGuard,
}

fn spawn_server(workdir: &std::path::Path) -> (FleetGuard, String) {
    let mut server = ServerGuard(
        Command::new(support::sibling_bin(support::SERVER_BIN))
            .env("HOME", workdir)
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
            let agent = ServerGuard(
                Command::new(support::sibling_bin(support::AGENT_BIN))
                    .env("HOME", workdir)
                    .arg("--workdir")
                    .arg(workdir)
                    .arg("--data-dir")
                    .arg(workdir.join("node-state"))
                    .args([
                        "--remote",
                        &base,
                        "--token",
                        TOKEN,
                        "--name",
                        "mode-switch-node",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("spawn node"),
            );
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let (_, nodes) = http(&base, "GET", "/api/nodes", "");
                // Scheduling (select_node) additionally requires a snapshot
                // with ready=true, so waiting for online alone races the
                // first POST /api/sessions into 503.
                if nodes["nodes"].as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|n| n["online"] == true && n["snapshot"]["ready"] == true)
                }) {
                    break;
                }
                assert!(Instant::now() < deadline, "node registration timed out");
                std::thread::sleep(Duration::from_millis(50));
            }
            return (
                FleetGuard {
                    _server: server,
                    _agent: agent,
                },
                base,
            );
        }
    }
}

fn http(base: &str, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {host}\r\nauthorization: Bearer {TOKEN}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    // `set_read_timeout` surfaces as ErrorKind::WouldBlock mid-body; under
    // workspace-wide load a 10s stall is normal, so keep reading instead of
    // panicking (bounded by an overall deadline).
    let mut response = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match stream.read_to_end(&mut response) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "http read stalled: {path}");
            }
            Err(e) => panic!("http read failed on {path}: {e}"),
        }
    }
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
    let stub = LlmStub::spawn(vec![Script::Hold]);
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".opencoder")).unwrap();
    std::fs::write(
        tmp.path().join(".opencoder/config.json"),
        format!("{{{}}}", stub.config_fragment()),
    )
    .unwrap();
    std::fs::write(tmp.path().join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let (_server, base) = spawn_server(tmp.path());

    let (status, created) = http(&base, "POST", "/api/sessions", r#"{"agent":"act"}"#);
    assert_eq!(status, 200, "session creation: {created}");
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
    const SID: &str = "operator-plan-clear-handoff-e2e";
    const PLAN: &str = "EXECUTE_DEPLOYMENT_PLAN_42";
    const RESULT: &str = "ACT_EXECUTION_COMPLETE_42";
    const RESUMED: &str = "RESUMED_ACT_COMPLETE_42";

    let stub = LlmStub::spawn_text(&[PLAN, RESULT, RESUMED]);
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".opencoder")).unwrap();
    std::fs::write(
        tmp.path().join(".opencoder/config.json"),
        format!("{{{}}}", stub.config_fragment()),
    )
    .unwrap();
    std::fs::write(tmp.path().join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let (server, base) = spawn_server(tmp.path());

    // Durable node placement precedes the first prompt. A title avoids the
    // unrelated automatic title-generation call.
    let created =
        serde_json::json!({"id":SID,"agent":"plan","title":"plan clear context"}).to_string();
    assert_eq!(http(&base, "POST", "/api/sessions", &created).0, 200);
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
