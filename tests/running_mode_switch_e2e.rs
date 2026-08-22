//! Real-binary regression: while an actual server drain is blocked in its LLM
//! request, dedicated switch endpoints (POST /agent, POST /handoff, the
//! `agent` field on /prompt) refuse with 409 and persist nothing, while
//! textual mode commands (/plan ...) are admitted and applied by the runner
//! at the idle boundary.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_opencoder");
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

struct ServerGuard(std::process::Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(workdir: &std::path::Path) -> (ServerGuard, String) {
    let mut server = ServerGuard(
        Command::new(BIN)
            .arg("--workdir")
            .arg(workdir)
            .args([
                "server",
                "--host",
                "127.0.0.1",
                "--port",
                "0",
                "--token",
                TOKEN,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = String::new();
    loop {
        assert!(Instant::now() < deadline, "server did not start");
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
