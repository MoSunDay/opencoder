//! Real-binary e2e smoke for `opencode server` + `opencode client` (review
//! TODO: solidify the manual act-session flag matrix into a script). Spawns
//! the actual `opencode` binary as a server on an ephemeral port and drives
//! the full client surface against it — session management subcommands,
//! operation flags (annotation / autopilot / interrupt), auth, client-side
//! validation, and a real (failing-fast) prompt run whose durable session
//! row survives the LLM failure. No real LLM: a local stub answers every
//! chat-completion request with HTTP 401 (non-retryable → immediate error).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_opencoder");
const TOKEN: &str = "smoke-bearer-token";

/// Minimal HTTP stub: every request gets an immediate 401 (non-retryable),
/// so the server's LLM call fails fast instead of burning the retry budget.
fn spawn_401_stub() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut s = stream;
                let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
                let _ = s.read(&mut [0u8; 8192]); // drain request head+body
                let body = br#"{"error":"stub auth denied"}"#;
                let head = format!(
                    "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(body);
            });
        }
    });
    port
}

/// Run the binary with a deadline; returns (success, stdout+stderr combined).
/// A hung child is killed so a regression fails instead of wedging the suite.
fn run(args: &[&str], timeout_secs: u64) -> (bool, String) {
    let mut child = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn opencode binary");
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                let mut out = String::new();
                let mut p = child.stdout.take().unwrap();
                let _ = p.read_to_string(&mut out);
                let mut p = child.stderr.take().unwrap();
                let _ = p.read_to_string(&mut out);
                return (status.success(), out);
            }
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return (false, "<timeout: killed>".into());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

struct ServerGuard(std::process::Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `opencode --workdir <wd> server --port 0 --token <T>` and parse the
/// printed listening URL from stdout.
fn spawn_server(workdir: &std::path::Path) -> (ServerGuard, String) {
    let mut child = Command::new(BIN)
        .arg("--workdir")
        .arg(workdir)
        .arg("server")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--token")
        .arg(TOKEN)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut url = String::new();
    'outer: loop {
        assert!(Instant::now() < deadline, "server never printed its URL");
        let mut buf = [0u8; 4096];
        let n = child.stdout.as_mut().unwrap().read(&mut buf).unwrap_or(0);
        url.push_str(&String::from_utf8_lossy(&buf[..n]));
        for line in url.lines() {
            if let Some(pos) = line.find("listening on http://") {
                url = line[pos + "listening on ".len()..].trim().to_string();
                break 'outer;
            }
        }
    }
    // Wait until the port actually accepts connections.
    let hostport = url.trim_start_matches("http://").to_string();
    loop {
        assert!(Instant::now() < deadline, "server never became reachable");
        if std::net::TcpStream::connect(&hostport).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (ServerGuard(child), url)
}

#[test]
fn client_server_flag_matrix_smoke() {
    let stub_port = spawn_401_stub();
    let workdir = std::env::temp_dir().join(format!(
        "oc-client-smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("config.json"),
        format!(
            r#"{{"model": "stub/m1", "providers": {{"stub": {{"base_url": "http://127.0.0.1:{stub_port}/v1", "api_key": "test-key", "model": "m1"}}}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();

    let (_guard, base) = spawn_server(&workdir);
    let wd = workdir.to_str().unwrap();
    let client = |args: &[&str]| {
        let mut v = vec![
            "--workdir",
            wd,
            "client",
            "--remote",
            &base,
            "--token",
            TOKEN,
        ];
        v.extend_from_slice(args);
        run(&v, 60)
    };

    // 1. Empty catalog: authenticated list succeeds and says so.
    let (ok, out) = client(&["session", "list"]);
    assert!(ok, "session list failed: {out}");
    assert!(out.contains("no sessions"), "unexpected list output: {out}");

    // 2. Auth: a wrong token is rejected.
    let bad = run(
        &[
            "--workdir",
            wd,
            "client",
            "--remote",
            &base,
            "--token",
            "wrong-token",
            "session",
            "list",
        ],
        30,
    );
    assert!(!bad.0, "wrong token must fail: {}", bad.1);

    // 3. Client-side validation: invalid autopilot, interrupt without session.
    let (ok, out) = client(&["--session", "zzz", "--autopilot", "bogus"]);
    assert!(!ok && out.contains("autopilot"), "invalid autopilot: {out}");
    let (ok, out) = client(&["--interrupt"]);
    assert!(
        !ok && out.contains("require --session"),
        "interrupt without resolution: {out}"
    );

    // 4. Real prompt run: the stub 401s the LLM call, so the run errors —
    // but the admitted prompt leaves a durable session row behind.
    let (ok, out) = client(&["--", "hello over the wire"]);
    assert!(!ok, "prompt run must fail against the 401 stub: {out}");
    assert!(out.contains("401"), "LLM failure must surface: {out}");
    let (ok, out) = client(&["session", "list"]);
    assert!(ok, "list after failed run: {out}");
    let id = out
        .lines()
        .find(|l| !l.contains("no sessions"))
        .and_then(|l| l.split('\t').next())
        .expect("session id")
        .trim()
        .to_string();

    // 5. Deep observation: show returns the full JSON state.
    let (ok, out) = client(&["session", "show", &id]);
    assert!(ok, "session show: {out}");
    assert!(out.contains("\"id\""), "show must be JSON: {out}");

    // 6. Configure-and-exit flags over --continue.
    let (ok, out) = client(&["--continue", "--annotation", "smoke note"]);
    assert!(ok, "annotation op: {out}");
    let (ok, out) = client(&["--continue", "--autopilot", "off"]);
    assert!(ok, "autopilot op: {out}");
    let (ok, out) = client(&["session", "show", &id]);
    assert!(
        ok && out.contains("smoke note"),
        "annotation persisted: {out}"
    );
    assert!(out.contains("off"), "autopilot persisted: {out}");

    // 7. Interrupt result feedback: nothing is draining → structured failure.
    let (ok, out) = client(&["--continue", "--interrupt"]);
    assert!(!ok && out.contains("no active"), "interrupt verdict: {out}");

    // 8. Questions surface.
    let (ok, out) = client(&["questions", "list", &id]);
    assert!(ok && out.contains("no questions"), "questions list: {out}");
    let (ok, out) = client(&["questions", "answer", &id, "nope", "x"]);
    assert!(!ok, "answering an unknown question must fail: {out}");

    // 9. Fork then delete cascade.
    let (ok, out) = client(&["session", "fork", &id]);
    assert!(ok, "fork: {out}");
    let fork_id = out.trim().to_string();
    assert!(
        !fork_id.is_empty() && fork_id != id,
        "fork prints new id: {out}"
    );
    let (ok, out) = client(&["session", "list"]);
    assert!(
        ok && out.lines().filter(|l| l.contains('\t')).count() == 2,
        "two sessions after fork: {out}"
    );
    for sid in [&id, &fork_id] {
        let (ok, out) = client(&["session", "delete", sid]);
        assert!(ok, "delete {sid}: {out}");
    }
    let (ok, out) = client(&["session", "list"]);
    assert!(
        ok && out.contains("no sessions"),
        "list empty after deletes: {out}"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}
