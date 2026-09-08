//! Full-chain e2e for the `opencoder-cli` surface against a REAL control
//! plane plus a scripted WebSocket node. `server_local.rs` proves the HTTP
//! contract node-less; this suite closes the relay gap by linking the
//! control e2e `MockNode` through the real fleet uplink
//! (`opencoder_node::fleet::run`) and driving the exact CLI entry points:
//!
//! * `exec create` reaches the node (its durable journal is the round-trip
//!   proof) and the persisted index reads back via `exec get`;
//! * SSE event streaming terminates once the node reports `finished`, and
//!   `--after` resume filtering holds both in-process and through the real
//!   `opencoder-cli` binary at process level (one compact JSON line per
//!   frame — the agent contract);
//! * artifact download streams exact bytes across the 64 KiB protocol
//!   chunk boundary;
//! * the exit-code contract (0 ok / 2 auth / 4 rejection) survives the
//!   process boundary on the streaming surface.
//!
//! Verified control semantics baked into the fixtures: execution ids must
//! carry their kind prefix (core `CreateExecution::validate`), and artifact
//! downloads require a dag/project execution (`api/streaming/artifact.rs`),
//! so the artifact leg creates a dag execution with an inline
//! `input.definition`.

#![allow(dead_code)]

mod server_local_defs;

#[path = "../../control/tests/e2e/support/node.rs"]
mod mock_node;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::task::JoinHandle;

use mock_node::MockNode;
use opencoder_node::fleet::NodeService;
use server_local_defs::{assert_ok, cli, Server, TOKEN};

/// Scripted node id; every create body pins it via `node_id`.
const NODE_ID: &str = "node-ctl";

/// One control plane + one scripted WS node per test. No shared global
/// state: each harness owns its tempdir workspace, ephemeral port and
/// fleet channel; `Drop` aborts both tasks and the tempdir drops last.
struct NodeHarness {
    server: Server,
    node: Arc<MockNode>,
    link: JoinHandle<()>,
}

impl NodeHarness {
    async fn new() -> Self {
        let server = Server::new(None).await;
        let node = MockNode::new(NODE_ID);
        let link = {
            let remote = server.base.clone();
            let service: Arc<dyn NodeService> = node.clone();
            tokio::spawn(async move {
                let _ = opencoder_node::fleet::run(&remote, TOKEN, service).await;
            })
        };
        wait_online(&server, &link).await;
        Self { server, node, link }
    }
}

impl Drop for NodeHarness {
    fn drop(&mut self) {
        self.link.abort();
    }
}

/// Block until the linked node completes its initial sync (online with a
/// ready snapshot); fails fast if the channel task died. 20s ceiling.
async fn wait_online(server: &Server, link: &JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            assert!(!link.is_finished(), "node channel exited");
            let ready = server
                .state
                .hub
                .views()
                .await
                .iter()
                .any(|n| n.online && n.snapshot.as_ref().is_some_and(|s| s.ready));
            if ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("node initial sync within 20s");
}

/// One journaled event row (shape shared with control's e2e suites).
fn row(seq: i64, kind: &str) -> Value {
    json!({"seq": seq, "kind": kind, "data": {"n": seq}, "ts": seq})
}

/// Minimal inline dag definition for artifact-capable executions.
fn dag_spec() -> Value {
    json!({"name": "ctl-relay", "steps": [
        {"name": "build", "kind": {"type": "wasm", "command": "tool.wasm"}}]})
}

/// Create body pinned to the scripted node (`kind` must match the id prefix).
fn create_body(id: &str, kind: &str, input: Value) -> String {
    json!({"id": id, "kind": kind, "input": input, "node_id": NODE_ID}).to_string()
}

/// `exec create` through the in-process CLI; 0 on the 202 receipt.
async fn create(h: &NodeHarness, id: &str, kind: &str, input: Value) -> i32 {
    let body = create_body(id, kind, input);
    cli(&h.server, TOKEN, &["exec", "create", "--json", &body]).await
}

// ── relay round-trip, SSE termination, artifact byte fidelity ──────────

#[tokio::test]
async fn exec_create_reaches_the_node_and_streams_events() {
    let h = NodeHarness::new().await;
    let s = &h.server;

    // Agent create: control placement relays the assignment over WS and
    // the node journals it — the round-trip proof.
    assert_eq!(
        create(&h, "agent-ctl-node-1", "agent", json!({"prompt": "hi"})).await,
        0
    );
    assert!(
        h.node
            .journal_ids()
            .contains(&"agent-ctl-node-1".to_string()),
        "create must reach the node journal, got {:?}",
        h.node.journal_ids()
    );
    // Inspect relays by id to the owning node (seeded table row).
    h.node.set_inspect(
        "agent-ctl-node-1",
        json!({"execution": {"id": "agent-ctl-node-1", "status": "idle"}}),
    );
    assert_ok(s, &["exec", "get", "agent-ctl-node-1"]).await;

    // `finished: true` makes control close the SSE stream after the rows,
    // so the CLI call terminates instead of following forever.
    h.node.set_events(
        "agent-ctl-node-1",
        vec![
            row(1, "llm_round_start"),
            row(2, "text_delta"),
            row(3, "done"),
        ],
        true,
    );
    let streamed = tokio::time::timeout(
        Duration::from_secs(20),
        cli(
            s,
            TOKEN,
            &["exec", "events", "agent-ctl-node-1", "--after", "0"],
        ),
    )
    .await
    .expect("SSE terminates once the node reports finished");
    assert_eq!(streamed, 0);
    // Buffered page with a resume cursor: rows strictly after seq 2.
    assert_ok(
        s,
        &["exec", "events-page", "agent-ctl-node-1", "--after", "2"],
    )
    .await;

    // Artifacts require a dag execution; 100_000 bytes force two 64 KiB
    // protocol chunks, so exact bytes prove chunked reassembly.
    assert_eq!(
        create(
            &h,
            "dag-ctl-node-1",
            "dag",
            json!({"definition": dag_spec()})
        )
        .await,
        0
    );
    assert_eq!(
        h.node.journal_ids().len(),
        2,
        "both creates reached the node"
    );
    let payload: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    h.node
        .set_artifact("dag-ctl-node-1", "build", "out.bin", payload.clone());
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("artifact.bin");
    let dest_arg = dest.to_str().unwrap().to_string();
    assert_eq!(
        cli(
            s,
            TOKEN,
            &[
                "exec",
                "artifact",
                "dag-ctl-node-1",
                "--step",
                "build",
                "--file",
                "out.bin",
                "-o",
                &dest_arg,
            ],
        )
        .await,
        0
    );
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        payload,
        "byte fidelity across the chunk boundary"
    );

    // The linked node is visible through the fleet view.
    assert_ok(s, &["nodes", "list"]).await;
}

// ── process-level SSE resume + the auth exit code ─────────────────────

#[tokio::test]
async fn sse_after_resume_and_exit_codes_process_level() {
    let h = NodeHarness::new().await;
    assert_eq!(
        create(&h, "agent-ctl-node-2", "agent", json!({"prompt": "hi"})).await,
        0
    );
    h.node.set_events(
        "agent-ctl-node-2",
        vec![
            row(1, "llm_round_start"),
            row(2, "text_delta"),
            row(3, "done"),
        ],
        true,
    );

    let args = ["exec", "events", "agent-ctl-node-2", "--after", "2"];
    let (code, stdout) = run_cli_binary(&h.server.base, TOKEN, &args).await;
    assert_eq!(code, 0, "stdout was: {stdout}");
    // Agent contract: every stdout line parses as one compact JSON frame;
    // the resume cursor drops seq 1..=2, leaving exactly the done frame.
    let frames: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("one compact JSON object per line"))
        .collect();
    let seqs: Vec<i64> = frames.iter().filter_map(|f| f["seq"].as_i64()).collect();
    assert_eq!(seqs, vec![3], "frames: {frames:?}");
    assert_eq!(frames[0]["event"], json!("done"));
    assert_eq!(frames[0]["data"], json!({"n": 3}));

    // Wrong bearer on the streaming surface: exit 2 past the process
    // boundary (auth classification, not a generic transport failure).
    let (code, _) = run_cli_binary(&h.server.base, "wrong-token", &args).await;
    assert_eq!(code, 2);
}

/// Run the real `opencoder-cli` binary (this crate's `[[bin]]`) as a child
/// process and collect (exit code, stdout). SSE closes on `finished`, so
/// `.output()` returns; the 20s timeout is only a safety net.
async fn run_cli_binary(base: &str, token: &str, args: &[&str]) -> (i32, String) {
    let program = env!("CARGO_BIN_EXE_opencoder-cli").to_string();
    let base = base.to_string();
    let token = token.to_string();
    let argv: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let run = tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new(&program)
            .arg("--server")
            .arg(&base)
            .arg("--token")
            .arg(&token)
            .args(&argv)
            .output()
            .expect("spawn opencoder-cli binary");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    });
    tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("binary exits: SSE closes on finished")
        .expect("spawn_blocking joined")
}
