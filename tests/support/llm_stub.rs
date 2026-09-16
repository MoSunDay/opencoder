//! Deterministic OpenAI-compatible streaming stub for the fleet e2e suites.
//!
//! One TcpListener on an ephemeral loopback port serves scripted replies in
//! FIFO order and records every JSON request body that crosses the wire, so
//! tests can assert on what a real session runner actually sent. Extracted
//! from `running_mode_switch_e2e.rs`, which now consumes this module.
//!
//! The script is strictly deterministic (no randomness, no sleeps): each
//! connection consumes exactly one entry. A [`Script::Hold`] parks the LLM
//! request inside the stub until [`LlmStub::release`] — the lever for
//! cancel/restart scenarios. When the script is exhausted an extra request
//! is answered with a fixed terminal reply so a stray compaction/summary
//! call can never wedge a test.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// One scripted LLM response.
#[allow(dead_code)]
pub enum Script {
    /// A normal streaming completion carrying this assistant text.
    Text(String),
    /// Accept the request, then hold the connection open until
    /// [`LlmStub::release`] is called (models an in-flight drain).
    Hold,
    /// Reply with a non-200 status (models an LLM outage).
    Fail(u16, String),
}

/// Reply text for requests beyond the script: unmistakable in transcripts,
/// and terminal so the session runner finishes instead of retrying forever.
pub const EXTRA_REPLY: &str = "e2e-stub-extra-reply";

pub struct LlmStub {
    port: u16,
    script: Arc<Mutex<VecDeque<Script>>>,
    requests: Arc<(Mutex<Vec<String>>, Condvar)>,
    entered: Arc<(Mutex<bool>, Condvar)>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl LlmStub {
    /// Bind an ephemeral loopback port and serve `script` in FIFO order.
    pub fn spawn(script: Vec<Script>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let entered = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let shared: Arc<Mutex<VecDeque<Script>>> = Arc::new(Mutex::new(script.into()));
        let requests_thread = requests.clone();
        let shared_thread = shared.clone();
        let entered_thread = entered.clone();
        let release_thread = release.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let requests = requests_thread.clone();
                let shared = shared_thread.clone();
                let entered = entered_thread.clone();
                let release = release_thread.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let body = read_http_body(&mut stream);
                    requests.0.lock().unwrap().push(body);
                    requests.1.notify_all();
                    let entry = {
                        let mut queue = shared.lock().unwrap();
                        queue.pop_front()
                    };
                    match entry {
                        Some(Script::Text(text)) => write_completion(&mut stream, &text),
                        // The entered flag flips before the held reply is
                        // written, so wait_until_entered observes the parked
                        // request and cancel/restart tests have a stable
                        // "drain is in flight" point.
                        Some(Script::Hold) => {
                            let mut held = entered.0.lock().unwrap();
                            *held = true;
                            entered.1.notify_all();
                            drop(held);
                            let mut allowed = release.0.lock().unwrap();
                            while !*allowed {
                                allowed = release
                                    .1
                                    .wait_timeout(allowed, Duration::from_millis(100))
                                    .unwrap()
                                    .0;
                            }
                            write_completion(&mut stream, EXTRA_REPLY);
                        }
                        Some(Script::Fail(status, message)) => {
                            write_failure(&mut stream, status, &message)
                        }
                        None => write_completion(&mut stream, EXTRA_REPLY),
                    }
                });
            }
        });
        Self {
            port,
            script: shared,
            requests,
            entered,
            release,
        }
    }

    /// Convenience: every script entry is a plain text reply.
    #[allow(dead_code)]
    pub fn spawn_text(texts: &[&str]) -> Self {
        Self::spawn(texts.iter().map(|t| Script::Text((*t).into())).collect())
    }

    /// Prepend a reply for a request that may or may not happen (e.g. an
    /// optional retry), without disturbing the rest of the script.
    #[allow(dead_code)]
    pub fn prepend(&self, entry: Script) {
        self.script.lock().unwrap().push_front(entry);
    }

    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The `<workdir>/.opencoder/config.json` fragment pointing the fleet at
    /// this stub: the injection path verified by `running_mode_switch_e2e`.
    #[allow(dead_code)]
    pub fn config_fragment(&self) -> String {
        format!(
            r#""model":"stub/m1","providers":{{"stub":{{"base_url":"http://127.0.0.1:{}/v1","api_key":"test-key","model":"m1"}}}}"#,
            self.port
        )
    }

    pub fn request_count(&self) -> usize {
        self.requests.0.lock().unwrap().len()
    }

    /// Block until `count` request bodies have been observed.
    pub fn wait_for_requests(&self, count: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(30);
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

    /// Block until a [`Script::Hold`] entry has consumed its request.
    pub fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
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

    /// Release a held request (idempotent, safe without a Hold entry).
    pub fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}

/// Read one HTTP request (headers + content-length body) off the wire.
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

/// Write a minimal OpenAI chat-completions SSE replay for `text`.
fn write_completion(stream: &mut TcpStream, text: &str) {
    let delta = serde_json::json!({"choices": [{"delta": {"content": text}}]});
    let body = format!(
        "data: {delta}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n",
        delta = delta
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

/// Write a non-200 JSON failure so the LLM client surfaces an error.
fn write_failure(stream: &mut TcpStream, status: u16, message: &str) {
    let body = serde_json::json!({"error": {"message": message}}).to_string();
    let head = format!(
        "HTTP/1.1 {} Failure\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        status,
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}
