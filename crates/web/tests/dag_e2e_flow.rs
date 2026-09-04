//! Process-level e2e for the DAG execution plane: the REAL node-side
//! runtime (`opencoder-dag-runtime::execute_run`) claims a dispatched run
//! from the REAL `build_app` server over signed HTTP, executes an `agent`
//! step on the real session runner with a scripted `MockChatClient`, and
//! the browser-visible projections converge: SSE event stream (uploaded
//! frames + the server's synthetic `run_finished`), run-row terminal
//! status, and the step's local artifacts.
//!
//! Cancel half: a run whose cancel flag is already flipped folds to
//! `cancelled` without ever starting a step.

mod node_e2e_support;
mod support;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use node_e2e_support::*;
use opencoder_dag::DagRunStatus;
use opencoder_dag_runtime::{execute_run, ExecDeps, RunDeps};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::uplink::Uplink;
use serde_json::{json, Value};

/// One `agent` step whose reply ends in a ```json fence (structured output).
fn spec_json() -> Value {
    json!({
        "name": "e2e-flow",
        "steps": [{
            "name": "analyze",
            "kind": { "type": "agent", "prompt": "给出结论" }
        }]
    })
}

/// register -> upsert def -> dispatch pinned to the node. Returns
/// (node_id, run_id).
async fn dispatch_run(base: &str, name: &str) -> (String, String) {
    let (_, b) = post_json(base, "/api/nodes/register", Some(json!({ "name": name }))).await;
    let node_id = b["node_id"].as_str().unwrap().to_string();
    let (_, d) = post_json(base, "/api/dag/defs", Some(json!({ "spec": spec_json() }))).await;
    let def_id = d["id"].as_str().unwrap().to_string();
    let uri = format!("/api/dag/defs/{def_id}/dispatch");
    let (s, r) = post_json(base, &uri, Some(json!({ "node_id": node_id }))).await;
    assert_eq!(s, reqwest::StatusCode::OK, "{r}");
    let rid = r["run_id"].as_str().unwrap().to_string();
    (node_id, rid)
}

/// Signed GET of the run row (terminal-state reconciliation side).
async fn run_view(base: &str, rid: &str) -> Value {
    let (s, v) = get_json(base, &format!("/api/dag/runs/{rid}")).await;
    assert_eq!(s, reqwest::StatusCode::OK, "{v}");
    v
}

/// The runtime's own `run_finished` frame, seen live on the SSE stream.
async fn await_sse_terminal(
    sse: &mut (impl futures::Stream<Item = Frame> + Unpin),
    want_status: &str,
) -> Vec<Frame> {
    let mut seen = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "no terminal frame in 20s"
        );
        let frame = tokio::time::timeout(Duration::from_secs(5), sse.next())
            .await
            .expect("sse frame within 5s")
            .expect("sse stream open");
        let is_terminal = frame.kind == "run_finished"
            && frame.data["payload"]["status"].as_str() == Some(want_status);
        let stop = is_terminal;
        seen.push(frame);
        if stop {
            return seen;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claimed_run_executes_and_converges_done_on_the_server() {
    let server = spawn_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let (node_id, rid) = dispatch_run(&server.base, "dag-e2e-done").await;

    // Subscribe BEFORE claiming: the live fanout must carry the run.
    let mut sse = open_sse(&server.base, &format!("/api/dag/runs/{rid}/events")).await;

    // Claim through the real signed worker uplink.
    let uplink = Arc::new(Uplink::new(&server.base, TOKEN).unwrap());
    let run = uplink
        .dag_claim(&node_id)
        .await
        .unwrap()
        .expect("run was due for the pinned node");
    assert_eq!(run.run_id, rid);

    let text = "结论如下\n```json\n{\"answer\": 42}\n```";
    let client = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta(text.to_string()),
        LlmEvent::Completed {
            text: text.to_string(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&uplink),
            exec: ExecDeps {
                store: Arc::clone(&server.store),
                client,
                workdir: tmp.path().to_path_buf(),
                config: opencoder_core::Config::load(tmp.path()).unwrap(),
            },
            workflow_root: tmp.path().join("workflow"),
        },
        run,
        cancel_rx,
    )
    .await
    .unwrap();
    assert_eq!(status, DagRunStatus::Done);

    // Run row converges to done with a finish timestamp.
    let view = run_view(&server.base, &rid).await;
    assert_eq!(view["status"], json!("done"));
    assert!(view["finished_at"].is_i64());

    // Live SSE carries the full uploaded projection in order.
    let frames = await_sse_terminal(&mut sse, "done").await;
    let kinds: Vec<&str> = frames.iter().map(|f| f.kind.as_str()).collect();
    let expected = ["run_started", "step_started", "step_done", "run_finished"];
    assert_eq!(&kinds[..expected.len()], &expected, "kinds={kinds:?}");
    let step_done = &frames[2];
    assert_eq!(step_done.data["step"], json!("analyze"));
    assert_eq!(step_done.data["payload"]["ok"], json!(true));
    // Uploaded frames carry the step's snapshot (transcript tail).
    assert!(step_done.data["payload"]["output"]
        .as_str()
        .unwrap()
        .contains("answer"));

    // Step artifacts landed under <workflow_root>/<run_id>/<step>/.
    let dir = tmp.path().join("workflow").join(&rid).join("analyze");
    let meta: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["outcome"], json!("done"));
    let output: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("output.json")).unwrap()).unwrap();
    assert_eq!(output, json!({ "answer": 42 }));

    // The step left a durable session row in the shared store (title pin).
    assert!(has_step_session(&server, &rid).await);
}

/// The step's session row is discoverable through the store the server
/// itself owns (title pin `dag/<run>/<step>`).
async fn has_step_session(server: &Server, rid: &str) -> bool {
    let sessions = server
        .store
        .list_sessions(&opencoder_store::SessionFilter::default())
        .await
        .unwrap();
    sessions
        .iter()
        .any(|s| s.title.as_deref() == Some(&format!("dag/{rid}/analyze")))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_cancelled_run_reports_cancelled_without_starting_a_step() {
    let server = spawn_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let (node_id, rid) = dispatch_run(&server.base, "dag-e2e-cancel").await;

    let mut sse = open_sse(&server.base, &format!("/api/dag/runs/{rid}/events")).await;

    let uplink = Arc::new(Uplink::new(&server.base, TOKEN).unwrap());
    let run = uplink.dag_claim(&node_id).await.unwrap().expect("run due");

    let (tx, cancel_rx) = tokio::sync::watch::channel(false);
    tx.send(true).unwrap();
    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&uplink),
            exec: ExecDeps {
                store: Arc::clone(&server.store),
                client: Arc::new(MockChatClient::new()),
                workdir: tmp.path().to_path_buf(),
                config: opencoder_core::Config::load(tmp.path()).unwrap(),
            },
            workflow_root: tmp.path().join("workflow"),
        },
        run,
        cancel_rx,
    )
    .await
    .unwrap();
    assert_eq!(status, DagRunStatus::Cancelled);

    // execute_run returns only after its status report was accepted, so the
    // durable row is already terminal here.
    let view = run_view(&server.base, &rid).await;
    assert_eq!(view["status"], json!("cancelled"));

    // SSE: run_started, a cancelled step_done (ok=false), run_finished.
    let frames = await_sse_terminal(&mut sse, "cancelled").await;
    let kinds: Vec<&str> = frames.iter().map(|f| f.kind.as_str()).collect();
    assert_eq!(kinds, vec!["run_started", "step_done", "run_finished"]);
    assert_eq!(frames[1].data["step"], json!("analyze"));
    assert_eq!(frames[1].data["payload"]["ok"], json!(false));

    // The step never started: no session row was created for it.
    assert!(!has_step_session(&server, &rid).await);
}
