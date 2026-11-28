//! Real-binary regression for the cross-process `todos interrupt` contract.
//!
//! A `todos run` / `resume` process drives a Running workflow for its whole
//! lifetime. The regression under test: `todos interrupt <id>` from a second
//! process used to open the store as a writer and be refused by the
//! per-database single-writer lock, leaving the user without a way to park
//! the workflow (a later fallback queued an intent file for the driver to
//! apply). The contract now is the historical one again, on the shared
//! multi-process WAL: the interrupt opens the database as one more writer
//! beside the live driver, commits the suspension itself, writes no intent
//! file, and reports success once the suspension is durable; the driver
//! notices the persisted state at its next step boundary and stops without a
//! second interruption event.
//!
//! Process-level form (rule 03, layer 2): the real `opencoder` binary in both
//! roles plus the loopback LLM stub from `tests/support`, which answers the
//! driver's parent decision and then parks the dispatched child session, so
//! the workflow is demonstrably Running under a live driver when the
//! interrupt arrives. Zero external services.

mod support;

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde_json::{json, Value};
use support::llm_stub::{LlmStub, Script};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc,
    task::JoinHandle,
};

/// Reason the interrupt command records; the suspension and its event carry it.
const REASON: &str = "CLI interrupt requested";
/// Upper bound for one CLI invocation. The interrupt commits a single
/// transition, so this only guards against a hung process.
const CLI_DEADLINE: Duration = Duration::from_secs(40);
/// Upper bound for the driver to reach an announced state (its id on stderr,
/// the workflow Running).
const ANNOUNCE_DEADLINE: Duration = Duration::from_secs(60);

/// Per-test sandbox: one tempdir holds the workdir, an isolated HOME and the
/// data root, so nothing leaks into the developer's real `~/.opencoder`.
struct Sandbox {
    _dir: tempfile::TempDir,
    workdir: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // `data_dir_for` digests the canonical workdir; hand every child the
        // same canonical path the assertions use below.
        let workdir = dir.path().canonicalize().unwrap();
        Self { _dir: dir, workdir }
    }

    /// `opencoder --workdir <sandbox> <args...>` with a cleared environment:
    /// only PATH, HOME and the XDG roots survive, so host overlays such as
    /// `OPENCODER_MODEL` / `OPENAI_BASE_URL` never reach the child.
    fn cli(&self, args: &[&str]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_opencoder"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.workdir)
            .env("XDG_DATA_HOME", self.workdir.join("data"))
            .env("XDG_CONFIG_HOME", self.workdir.join("config-home"))
            .current_dir(&self.workdir)
            .kill_on_drop(true)
            .arg("--workdir")
            .arg(&self.workdir)
            .args(args);
        command
    }

    /// The per-workdir data directory every child resolves for the workdir:
    /// `<XDG_DATA_HOME>/opencoder/<digest(workdir)>` with the sandbox's data
    /// root, the same layout `data_dir_for` derives inside the children.
    fn data_dir(&self) -> PathBuf {
        self.workdir
            .join("data")
            .join("opencoder")
            .join(opencoder_core::workdir_hash(&self.workdir))
    }
}

/// Every file under the data directory's `todos/.interrupts`, the intent
/// channel an older build used. The direct path must leave it empty.
fn intent_files(data_dir: &Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("todos").join(".interrupts")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok())
        .collect()
}

/// The intent file path an older build would have written for `workflow_id`:
/// `<data_dir>/todos/.interrupts/<id>.json` (run ids need no encoding).
fn intent_path(data_dir: &Path, workflow_id: &str) -> PathBuf {
    data_dir
        .join("todos")
        .join(".interrupts")
        .join(format!("{workflow_id}.json"))
}

/// Write the one-todo spec the run command drives and return its path.
fn write_spec(workdir: &Path) -> PathBuf {
    let path = workdir.join("case.json");
    std::fs::write(
        &path,
        json!({
            "schema_version": 1,
            "id": "wf-interrupt-e2e",
            "name": "interrupt e2e",
            "objective": "verify the cross-process interrupt contract",
            "constraints": [],
            "todos": [{
                "id": "t1",
                "title": "single todo",
                "requirement_background": "required by the spec",
                "instructions": "write a candidate file",
                "depends_on": [],
                "agent": "act",
                "max_attempts": 1,
                "acceptance": {"criteria": "the file exists", "required_tool_calls": []}
            }],
            "metadata": {}
        })
        .to_string(),
    )
    .unwrap();
    path
}

/// Pump one child stream line by line into `sink`; the task ends with the
/// pipe. Keeps the child from blocking on a full pipe while the test only
/// inspects the lines it cares about.
fn pump<R>(reader: R, sink: mpsc::UnboundedSender<String>) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if sink.send(line).is_err() {
                return;
            }
        }
    })
}

/// The running driver: `todos run` against the stub, with both output
/// streams pumped into one transcript. Returns the child, the transcript
/// receiver and the workflow id the run announced on stderr.
async fn spawn_run(
    sandbox: &Sandbox,
    spec: &Path,
    stub: &LlmStub,
) -> (
    tokio::process::Child,
    mpsc::UnboundedReceiver<String>,
    String,
) {
    support::fleet_proc::write_config(&sandbox.workdir, stub.port(), json!({}));
    let mut child = sandbox
        .cli(&["todos", "run", "--file", &spec.to_string_lossy()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn todos run");
    let (tx, mut rx) = mpsc::unbounded_channel();
    pump(child.stdout.take().unwrap(), tx.clone());
    pump(child.stderr.take().unwrap(), tx);
    let deadline = tokio::time::Instant::now() + ANNOUNCE_DEADLINE;
    loop {
        // `todos run` announces its generated id on stderr before the first
        // parent decision; progress lines may interleave, so scan the stream.
        let line = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("todos run never announced its workflow id")
            .expect("todos run exited before announcing its workflow id");
        if let Some(id) = line.strip_prefix("workflow_id=") {
            return (child, rx, id.trim().to_string());
        }
    }
}

/// Run one CLI invocation to completion under [`CLI_DEADLINE`].
async fn run_to_completion(mut command: tokio::process::Command) -> std::process::Output {
    tokio::time::timeout(CLI_DEADLINE, command.kill_on_drop(true).output())
        .await
        .expect("CLI invocation timed out")
        .expect("CLI invocation failed to run")
}

/// Run a read-only `todos` command and parse its single JSON document.
async fn read_json(sandbox: &Sandbox, args: &[&str]) -> Value {
    let output = run_to_completion(sandbox.cli(args)).await;
    assert!(
        output.status.success(),
        "`{args:?}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON document")
}

/// The parent decision the stub scripts: dispatch the one TODO into a new
/// child session, exactly as `parent::schedule` parses it.
fn dispatch_decision() -> String {
    json!({
        "operation": "dispatch",
        "todos": [{"todo_id": "t1", "context_mode": "new"}],
        "reason": "ready"
    })
    .to_string()
}

#[tokio::test]
async fn interrupt_suspends_the_workflow_a_run_process_is_driving() {
    let sandbox = Sandbox::new();
    let spec = write_spec(&sandbox.workdir);
    let stub = LlmStub::spawn(vec![Script::Text(dispatch_decision()), Script::Hold]);

    // 1. The driver starts, dispatches the TODO and parks inside the child
    //    session: the workflow is Running under a live driver.
    let (mut run, _transcript, workflow_id) = spawn_run(&sandbox, &spec, &stub).await;
    stub.wait_until_entered();
    let deadline = tokio::time::Instant::now() + ANNOUNCE_DEADLINE;
    loop {
        let shown = read_json(&sandbox, &["todos", "show", &workflow_id, "--json"]).await;
        if shown["state"]["status"] == "running" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the run never moved the workflow to running: {shown}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 2. A second process interrupts. Before the contract closure this was
    //    refused by the single-writer lock; now the command commits the
    //    suspension as one more writer beside the live driver and reports it.
    let interrupt = run_to_completion(sandbox.cli(&["todos", "interrupt", &workflow_id])).await;
    let stdout = String::from_utf8_lossy(&interrupt.stdout);
    let stderr = String::from_utf8_lossy(&interrupt.stderr);
    assert!(
        interrupt.status.success(),
        "interrupt must succeed against a live driver; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(&format!("{workflow_id} suspended at generation")),
        "interrupt must report the suspension; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("queued"),
        "a committed interrupt is never reported as queued: {stdout}"
    );

    // 3. The command wrote no intent: the suspension lives in the database
    //    alone and is already visible to other processes.
    let intents = intent_files(&sandbox.data_dir());
    assert!(
        intents.is_empty(),
        "the direct path writes no intent: {intents:?}"
    );
    assert!(
        !intent_path(&sandbox.data_dir(), &workflow_id).exists(),
        "no intent file for the workflow"
    );
    let shown = read_json(&sandbox, &["todos", "show", &workflow_id, "--json"]).await;
    assert_eq!(shown["state"]["status"], "suspended", "{shown}");
    assert_eq!(shown["state"]["terminal_reason"], REASON, "{shown}");

    // 4. The driver notices the persisted suspension and exits. A suspension
    //    that is not a local Ctrl-C ends the run with the status name, so the
    //    exit code is not part of the contract; terminating is.
    stub.release();
    let status = tokio::time::timeout(CLI_DEADLINE, run.wait())
        .await
        .expect("the interrupted driver must exit")
        .expect("wait for the driver");
    assert!(
        status.code().is_some(),
        "the driver must exit by itself, not on a signal: {status}"
    );

    // 5. Exactly one interruption, the one the interrupt command committed:
    //    the driver adopted the persisted state instead of adding its own.
    let shown = read_json(&sandbox, &["todos", "show", &workflow_id, "--json"]).await;
    assert_eq!(shown["state"]["status"], "suspended", "{shown}");
    assert_eq!(shown["state"]["terminal_reason"], REASON, "{shown}");

    let events = read_json(&sandbox, &["todos", "events", &workflow_id, "--json"]).await;
    let interrupted = events
        .as_array()
        .expect("events --json is a list")
        .iter()
        .filter(|event| event["kind"] == "workflow_interrupted")
        .collect::<Vec<_>>();
    assert_eq!(interrupted.len(), 1, "one interruption event: {events}");
    assert_eq!(interrupted[0]["payload"]["reason"], REASON);
    assert!(
        interrupted[0]["payload"].get("request_id").is_none(),
        "the direct path records no intent request id: {interrupted:?}"
    );
}
