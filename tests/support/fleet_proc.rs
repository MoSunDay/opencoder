//! Spawn the real fleet (`opencoder-server` + `opencoder-agent`) as child
//! processes for the e2e suites, with a loopback LLM stub wired in through
//! `<workdir>/.opencoder/config.json`.
//!
//! Contract shared by every scenario:
//! - server: `--port 0` (base URL parsed from the `listening on` stdout
//!   line), `HOME`/`XDG_DATA_HOME` redirected into the test tempdir so the
//!   per-workdir store never touches the developer's real data dir;
//! - agent: its own `--data-dir` under the same tempdir (the node disk
//!   layout is an assertion surface), fixed token, `--name` set;
//! - readiness: the node must be `online` with `snapshot.ready == true` AND
//!   carry the expected execution kinds — waiting for `online` alone races
//!   the first POST into a 503;
//! - cleanup: RAII kill+wait on drop; when a test panics, the captured
//!   process output tail is printed so the failure stays diagnosable.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::http_util::{http, wait_until};
use super::{sibling_bin, AGENT_BIN, SERVER_BIN};

pub const TOKEN: &str = "fleet-e2e-token";

/// Joined stdout+stderr of both processes. Reader threads append; nothing
/// is lost to a full pipe buffer and failures print this tail.
#[derive(Clone, Default)]
pub struct ProcLog(Arc<Mutex<String>>);

impl ProcLog {
    fn push(&self, text: &str) {
        let mut log = self.0.lock().unwrap();
        log.push_str(text);
        if !log.ends_with('\n') {
            log.push('\n');
        }
        // Bound memory: keep only the tail; failures print this tail.
        if log.len() > 256 * 1024 {
            let cut = log.len() - 128 * 1024;
            let boundary = log[cut..].find('\n').map(|i| cut + i + 1).unwrap_or(cut);
            *log = log[boundary..].to_string();
        }
    }

    fn contains(&self, needle: &str) -> bool {
        self.0.lock().unwrap().contains(needle)
    }

    /// Last `lines` lines, for panic forensics.
    pub fn tail(&self, lines: usize) -> String {
        let log = self.0.lock().unwrap();
        let collected: Vec<&str> = log.lines().collect();
        collected
            .iter()
            .rev()
            .take(lines)
            .rev()
            .fold(String::new(), |acc, line| format!("{acc}{line}\n"))
    }
}

/// One child process with RAII kill+wait. stdout/stderr are drained by
/// detached reader threads into the shared [`ProcLog`].
struct Proc {
    child: Child,
}

impl Proc {
    fn spawn(mut command: Command, label: &str, log: &ProcLog) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {label} failed: {error}"));
        let out: ChildStdout = child.stdout.take().unwrap();
        let err: ChildStderr = child.stderr.take().unwrap();
        spawn_reader(out, label, "out", log);
        spawn_reader(err, label, "err", log);
        Self { child }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Drain one child stdio stream into the shared log.
fn spawn_reader(stream: impl Read + Send + 'static, label: &str, kind: &str, log: &ProcLog) {
    let prefix = format!("[{label} {kind}] ");
    let log = log.clone();
    std::thread::spawn(move || {
        let mut stream = stream;
        let mut buffer = [0u8; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => return,
                Ok(count) => {
                    let text = String::from_utf8_lossy(&buffer[..count]);
                    for line in text.lines() {
                        log.push(&format!("{prefix}{line}"));
                    }
                }
                Err(_) => return,
            }
        }
    });
}

/// Write `<workdir>/.opencoder/config.json`: the loopback LLM stub plus any
/// extra config keys (e.g. `dag.wasm_dir` for the DAG suites). Both fleet
/// processes discover this file through `--workdir`.
pub fn write_config(workdir: &Path, stub_port: u16, extra: Value) {
    let mut config = json!({
        "model": "stub/m1",
        "dag": {"wasm_dir": workdir.join("wasm-pool")},
        "providers": {
            "stub": {
                "base_url": format!("http://127.0.0.1:{stub_port}/v1"),
                "api_key": "test-key",
                "model": "m1",
            }
        },
    });
    if let (Some(base), Some(extra)) = (config.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    let dir = workdir.join(".opencoder");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
}

/// The live fleet: one server plus one (respawnable) agent process.
pub struct Fleet {
    /// Server base URL, e.g. `http://127.0.0.1:41234`.
    pub base: String,
    /// The test tempdir every process runs against.
    pub workdir: PathBuf,
    /// The agent's `--data-dir` (`<workdir>/node-state`): node disk layout.
    pub node_data: PathBuf,
    pub log: ProcLog,
    _server: Proc,
    agent: Option<Proc>,
    node_name: String,
}

impl Fleet {
    /// Spawn the fleet with a plain LLM-stub config and no extra keys.
    pub fn spawn(workdir: &Path, stub_port: u16, node_name: &str) -> Self {
        Self::spawn_with_config(workdir, stub_port, json!({}), node_name)
    }

    /// Spawn the fleet with extra config keys merged into the shared file.
    pub fn spawn_with_config(
        workdir: &Path,
        stub_port: u16,
        extra: Value,
        node_name: &str,
    ) -> Self {
        write_config(workdir, stub_port, extra);
        let log = ProcLog::default();
        let server = spawn_server(workdir, &log);
        let base = wait_until(&log, "opencoder-server to print `listening on`", 60, || {
            if log.contains("listening on http://") {
                let line = log
                    .tail(400)
                    .lines()
                    .rev()
                    .find(|line| line.contains("listening on "))
                    .unwrap_or("")
                    .to_string();
                parse_base(&line)
            } else {
                None
            }
        });
        let agent = spawn_agent(workdir, &base, &log, node_name);
        let fleet = Self {
            base,
            workdir: workdir.to_path_buf(),
            node_data: workdir.join("node-state"),
            log,
            _server: server,
            agent: Some(agent),
            node_name: node_name.to_string(),
        };
        fleet.wait_ready(&["operator", "dag"]);
        fleet
    }

    /// Poll `/api/nodes` until this fleet's node is online, ready and serves
    /// every kind in `kinds`; returns the durable node id.
    pub fn wait_ready(&self, kinds: &[&str]) -> String {
        wait_until(
            &self.log,
            &format!("node ready with kinds {kinds:?}"),
            60,
            || {
                let (_, nodes) = self.http("GET", "/api/nodes", &json!({}));
                for node in nodes["nodes"].as_array()? {
                    let kinds_ok = node["kinds"].as_array().is_some_and(|rows| {
                        kinds.iter().all(|kind| rows.iter().any(|k| k == kind))
                    });
                    if node["online"] == json!(true)
                        && node["snapshot"]["ready"] == json!(true)
                        && kinds_ok
                    {
                        return node["id"].as_str().map(str::to_string);
                    }
                }
                None
            },
        )
    }

    /// The node id (persisted across agent restarts in `<data>/node-id`).
    pub fn node_id(&self) -> String {
        self.wait_ready(&["operator", "dag"])
    }

    /// Bearer-authenticated JSON request against this server.
    pub fn http(&self, method: &str, path: &str, body: &Value) -> (u16, Value) {
        http(&self.base, method, path, TOKEN, &body.to_string())
    }

    /// JSON request with an explicit bearer (user-role scenarios).
    pub fn http_as(&self, method: &str, path: &str, token: &str, body: &Value) -> (u16, Value) {
        http(&self.base, method, path, token, &body.to_string())
    }

    /// Poll one execution until `done` accepts its inspect document.
    pub fn wait_status(
        &self,
        id: &str,
        label: &str,
        secs: u64,
        done: impl Fn(&Value) -> bool,
    ) -> Value {
        wait_until(&self.log, label, secs, || {
            let (status, body) = self.http("GET", &format!("/api/executions/{id}"), &json!({}));
            if status != 200 {
                return None;
            }
            done(&body).then_some(body)
        })
    }

    /// Wait for a terminal (`done`/`error`/`cancelled`) execution status.
    pub fn wait_terminal(&self, id: &str) -> Value {
        self.wait_status(id, &format!("terminal status for {id}"), 180, |body| {
            matches!(
                body["execution"]["status"].as_str(),
                Some("done") | Some("error") | Some("cancelled")
            )
        })
    }

    /// Wait for the operator session drained (`idle` — the operator's
    /// success state: a live chat session, not a terminal status).
    pub fn wait_idle(&self, id: &str) -> Value {
        self.wait_status(id, &format!("idle status for {id}"), 180, |body| {
            body["execution"]["status"] == json!("idle")
        })
    }

    /// Kill the agent process (models a node crash). The persisted node id
    /// survives in `<data>/node-id`.
    pub fn kill_agent(&mut self) {
        if let Some(mut agent) = self.agent.take() {
            agent.kill();
        }
    }

    /// Respawn the agent against the same `--data-dir` (restart recovery
    /// scenarios) and wait for it to be ready again.
    pub fn respawn_agent(&mut self) {
        self.kill_agent();
        self.agent = Some(spawn_agent(
            &self.workdir,
            &self.base,
            &self.log,
            &self.node_name,
        ));
        self.wait_ready(&["operator", "dag"]);
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("--- fleet e2e failure log tail ---\n{}", self.log.tail(80));
        }
        // Proc drops kill both children.
    }
}

fn spawn_server(workdir: &Path, log: &ProcLog) -> Proc {
    let mut command = Command::new(sibling_bin(SERVER_BIN));
    command
        .env("HOME", workdir)
        .arg("--workdir")
        .arg(workdir)
        .args(["--host", "127.0.0.1", "--port", "0", "--token", TOKEN])
        // Keep the per-workdir SQLite store inside the test's tempdir.
        .env("XDG_DATA_HOME", workdir.join("xdg"));
    Proc::spawn(command, "server", log)
}

fn spawn_agent(workdir: &Path, base: &str, log: &ProcLog, name: &str) -> Proc {
    let mut command = Command::new(sibling_bin(AGENT_BIN));
    command
        .env("HOME", workdir)
        .arg("--workdir")
        .arg(workdir)
        .arg("--data-dir")
        .arg(workdir.join("node-state"))
        .args(["--remote", base, "--token", TOKEN, "--name", name]);
    Proc::spawn(command, "agent", log)
}

/// Extract the `http://127.0.0.1:<port>` URL from a `listening on` line.
fn parse_base(line: &str) -> Option<String> {
    let rest = line.split("listening on ").nth(1)?;
    let url = rest.lines().next()?.trim().trim_end_matches('/');
    url.starts_with("http://").then(|| url.to_string())
}
