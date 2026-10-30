//! Shared process plumbing for this suite: resolve the sibling
//! `opencoder-server` binary, spawn it with a scrubbed environment (no
//! inherited token or proxy settings; `HOME`/`XDG_DATA_HOME` redirected
//! into the per-test tempdir), capture stdout+stderr line-by-line, wait
//! for the `listening on http://` readiness line or an HTTP health probe,
//! and always kill+reap on drop so a failing assert leaks no processes.

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::support::http_util::http;

/// Resolve the sibling `opencoder-server` binary (see [`crate::support`]).
pub fn server_bin() -> PathBuf {
    crate::support::sibling_bin(crate::support::SERVER_BIN)
}

/// One free loopback port (bind `:0`, read it back, drop the listener).
pub fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind 127.0.0.1:0")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Bounded shared stdout+stderr log: reader threads append whole lines,
/// failure messages print the tail, memory capped.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<String>>);

impl Log {
    fn push(&self, line: &str) {
        let mut log = self.0.lock().unwrap();
        log.push_str(line);
        if !log.ends_with('\n') {
            log.push('\n');
        }
        if log.len() > 256 * 1024 {
            let cut = log.len() - 128 * 1024;
            let boundary = log[cut..].find('\n').map(|i| cut + i + 1).unwrap_or(cut);
            *log = log[boundary..].to_string();
        }
    }

    /// Last `lines` lines, oldest first — the failure-message tail.
    pub fn tail(&self, lines: usize) -> String {
        let log = self.0.lock().unwrap();
        let collected: Vec<&str> = log.lines().collect();
        let start = collected.len().saturating_sub(lines);
        let mut text = String::new();
        for line in &collected[start..] {
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    /// Whether the captured output contains `needle` (readiness probes).
    pub fn contains(&self, needle: &str) -> bool {
        self.0.lock().unwrap().contains(needle)
    }
}

/// One running `opencoder-server` child: stdout/stderr drained by detached
/// reader threads into [`Log`], RAII kill+wait on drop, pollable exit.
pub struct ServerProc {
    log: Log,
    child: Arc<Mutex<Child>>,
}

impl ServerProc {
    /// Spawn the server binary. `OPENCODER_SERVER_TOKEN` and proxy vars are
    /// scrubbed first (`envs` applies after, so a test can deliberately set
    /// the env token); `HOME`/`XDG_DATA_HOME` redirect under `workdir` so
    /// the per-workdir store never touches the developer's real data dir.
    pub fn spawn(workdir: &Path, args: &[&str], envs: &[(&str, String)]) -> Self {
        let mut command = Command::new(server_bin());
        command
            .env_remove("OPENCODER_SERVER_TOKEN")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env("HOME", workdir)
            .env("XDG_DATA_HOME", workdir.join("xdg"))
            .args(args);
        for (name, value) in envs {
            command.env(name, value);
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn opencoder-server");
        let log = Log::default();
        spawn_reader(child.stdout.take().expect("piped stdout"), "out", &log);
        spawn_reader(child.stderr.take().expect("piped stderr"), "err", &log);
        Self {
            log,
            child: Arc::new(Mutex::new(child)),
        }
    }

    /// The captured output log (readiness probes, failure tails).
    pub fn log(&self) -> &Log {
        &self.log
    }

    /// The child PID (signal-delivery scenarios).
    pub fn pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }

    /// Poll until the platform server prints its
    /// `listening on http://127.0.0.1:<port>` line; returns the base URL.
    pub fn wait_http_ready(&self, secs: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if self.log.contains("listening on http://") {
                return self
                    .log
                    .tail(400)
                    .lines()
                    .rev()
                    .find_map(parse_base)
                    .expect("listening line parses");
            }
            assert!(
                Instant::now() < deadline,
                "no `listening on` line within {secs}s\n--- server log tail ---\n{}",
                self.log.tail(40)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Poll until the child exits; returns its real exit status.
    pub fn wait_exit(&self, secs: u64) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(status) = self.child.lock().unwrap().try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit within {secs}s\n--- server log tail ---\n{}",
                self.log.tail(40)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for ServerProc {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Drain one child stdio stream into the shared log, one line at a time.
fn spawn_reader(stream: impl Read + Send + 'static, kind: &'static str, log: &Log) {
    let log = log.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            match line {
                Ok(line) => log.push(&format!("[{kind}] {line}")),
                Err(_) => return,
            }
        }
    });
}

/// Extract the `http://127.0.0.1:<port>` URL from a `listening on` line.
fn parse_base(line: &str) -> Option<String> {
    let rest = line.split("listening on ").nth(1)?;
    let url = rest.lines().next()?.trim().trim_end_matches('/');
    url.starts_with("http://").then(|| url.to_string())
}

/// Poll `GET /api/health` with `token` until 200 — the readiness probe for
/// the resource service modes, which bind silently (no stdout line) and
/// refuse connections until the listener is up.
pub fn wait_health(log: &Log, base: &str, token: &str, secs: u64) {
    let host = base.trim_start_matches("http://").to_string();
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let listening = std::net::TcpStream::connect(&host).is_ok();
        if listening && http(base, "GET", "/api/health", token, "").0 == 200 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "/api/health never answered 200\n--- server log tail ---\n{}",
            log.tail(40)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
