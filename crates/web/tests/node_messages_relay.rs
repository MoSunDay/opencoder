//! P3 message relay: the browser asks the SERVER for a dialog that lives on a
//! worker node's LOCAL store. The server queues a `fetch_messages` control
//! task for the node, the worker answers from its own database, and the
//! server echoes the slice back — without persisting anything durable-side.
//!
//! Process-level e2e over the REAL router + REAL worker
//! (`opencode_node::run_node`, mock LLM), same harness as the fleet flow
//! tests. The worker-facing half (slice selection, local-store reads,
//! claim/heartbeat delivery, dedup) is covered by `runner_control.rs` and the
//! `control.rs` unit tests; this file pins the HTTP contract:
//!   200 slice / 404 unknown node / 400 empty session / 504 timeout /
//!   502 worker failure / `resolved:false` on late uploads / dialogs index /
//!   dispatch session reuse (`session_id` honored or 400).

mod node_e2e_support;
mod support;

use std::sync::Arc;

use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_store::SessionFilter;

use node_e2e_support::{get_json, post_json, spawn_node, wait_for, Server, TOKEN};

const NODE_NAME: &str = "relay-node";

fn script(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        },
    ]
}

async fn spawn_server_with_store() -> Server {
    // `Server` implements Drop (graceful shutdown) — hand the whole thing out.
    node_e2e_support::spawn_server().await
}

async fn register(base: &str, name: &str) -> String {
    let (_, b) = post_json(
        base,
        "/api/nodes/register",
        Some(serde_json::json!({ "name": name })),
    )
    .await;
    b["node_id"].as_str().unwrap().to_string()
}

async fn relay(
    base: &str,
    node_id: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let path = format!("/api/nodes/{node_id}/messages");
    let json = serde_json::to_vec(&body).unwrap();
    let (tsh, ts, sigh, sig) = support::sig_headers(TOKEN, "POST", &path, &json);
    let r = node_e2e_support::http()
        .post(format!("{base}{path}"))
        .header(tsh, ts)
        .header(sigh, sig)
        .header("content-type", "application/json")
        .body(json)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
    (status, v)
}

async fn upload_result(
    base: &str,
    node_id: &str,
    result: &opencoder_core::node_protocol::FetchMessagesResult,
) -> (reqwest::StatusCode, serde_json::Value) {
    let path = format!("/api/nodes/{node_id}/control_result");
    let json = serde_json::to_vec(result).unwrap();
    let (tsh, ts, sigh, sig) = support::sig_headers(TOKEN, "POST", &path, &json);
    let r = node_e2e_support::http()
        .post(format!("{base}{path}"))
        .header(tsh, ts)
        .header(sigh, sig)
        .header("content-type", "application/json")
        .body(json)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
    (status, v)
}

/// Register node + run one real task on a live worker so the node's LOCAL
/// store holds a dialog, and return (base, store, node_id, session_id).
type Runner = tokio::task::JoinHandle<anyhow::Result<()>>;

/// Keep-alive guard for the worker directories of [`run_one_task`].
struct Dirs {
    #[allow(dead_code)]
    workdir: tempfile::TempDir,
    #[allow(dead_code)]
    data: tempfile::TempDir,
}

/// Server + live worker + the node id + the dialog id created by one executed
/// task. The caller keeps the runner AND the worker directories alive until
/// its assertions are done (dropping `Dirs` deletes the worker's local db).
async fn run_one_task() -> (Server, String, String, Runner, Dirs) {
    let srv = spawn_server_with_store().await;
    let base = srv.base.clone();
    let store = srv.store.clone();
    let workdir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    node_e2e_support::pin_autopilot_off(workdir.path());

    let client = Arc::new(MockChatClient::new());
    let runner = spawn_node(
        &base,
        NODE_NAME,
        workdir.path(),
        data.path(),
        client.clone(),
    )
    .await;
    let node_id = wait_for(15, 100, || async {
        let (_, v) = get_json(&base, "/api/nodes").await;
        v["nodes"]
            .as_array()
            .and_then(|a| a.iter().find(|n| n["name"] == *NODE_NAME))
            .filter(|n| n["status"] == "idle")
            .map(|n| n["id"].as_str().unwrap().to_string())
    })
    .await;

    client.queue_script(script("relay hello"));
    let (_, d) = post_json(
        &base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": "relay me" })),
    )
    .await;
    let sid = d["session_id"].as_str().unwrap().to_string();
    let tid = d["task_id"].as_str().unwrap().to_string();
    // DETERMINISTIC completion gate: the durable task row only flips to
    // `Done` after the worker's terminal status upload, which itself follows
    // the local event flush — so the node-side transcript is guaranteed
    // readable by the time this returns. (Waiting for node "idle" here would
    // race: an idle node has not necessarily claimed the task yet.)
    wait_for(15, 50, || async {
        match store.get_node_task(&tid).await.unwrap() {
            Some(r) if r.status == opencoder_store::NodeTaskStatus::Done => Some(()),
            Some(r)
                if matches!(
                    r.status,
                    opencoder_store::NodeTaskStatus::Error
                        | opencoder_store::NodeTaskStatus::Cancelled
                ) =>
            {
                panic!("task ended {:#?}", r.status)
            }
            _ => None,
        }
    })
    .await;
    (srv, node_id, sid, runner, Dirs { workdir, data })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_echoes_worker_slice_and_persists_nothing() {
    let (srv, node_id, sid, runner, _dirs) = run_one_task().await;
    let (base, store) = (srv.base.clone(), srv.store.clone());

    let sessions_before = store
        .list_sessions(&SessionFilter {
            limit: 100,
            include_subagents: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .len();
    let tasks_before = store.list_node_tasks(&node_id, 200).await.unwrap().len();

    let (st, v) = relay(&base, &node_id, serde_json::json!({ "session_id": sid })).await;
    assert_eq!(st.as_u16(), 200, "{v}");
    assert_eq!(v["session_id"], sid.as_str());
    // No compaction has ever run on this dialog: the summary pair is null and
    // the slice starts at seq 1.
    assert!(v["summary"].is_null());
    assert!(v["summary_seq"].is_null());
    let msgs = v["messages"].as_array().unwrap();
    assert!(msgs.len() >= 2, "messages={msgs:?}");
    assert_eq!(msgs[0]["seq"], 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["blocks"][0]["kind"], "text");
    assert!(msgs[0]["created_at"].is_i64());
    assert!(
        msgs.iter().any(|m| m["role"] == "assistant"),
        "assistant turn missing: {msgs:?}"
    );

    // Nothing durable moved: no new session, no extra node-task row.
    let sessions_after = store
        .list_sessions(&SessionFilter {
            limit: 100,
            include_subagents: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .len();
    let tasks_after = store.list_node_tasks(&node_id, 200).await.unwrap().len();
    assert_eq!(sessions_before, sessions_after, "relay leaked a session");
    assert_eq!(tasks_before, tasks_after, "relay leaked a node task");

    // Dialogs index: the worker's dialog shows up grouped by session.
    let (_, dv) = get_json(&base, &format!("/api/nodes/{node_id}/dialogs")).await;
    let dialogs = dv["dialogs"].as_array().unwrap();
    let own = dialogs
        .iter()
        .find(|d| d["session_id"] == sid.as_str())
        .expect("dialog missing from index");
    assert!(own["task_count"].as_u64().unwrap() >= 1);
    assert!(own["last_created_at"].is_i64());
    assert!(own["first_created_at"].is_i64());
    runner.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_validates_node_and_session_and_times_out() {
    let srv = spawn_server_with_store().await;
    let base = srv.base.clone();

    // Unknown node -> 404; empty session_id -> 400 (node exists or not).
    let (st, _) = relay(
        &base,
        "01AAAAAAAAAAAAAAAAAAAAAAAA",
        serde_json::json!({ "session_id": "s" }),
    )
    .await;
    assert_eq!(st.as_u16(), 404);
    let node_id = register(&base, "ghost").await;
    let (st, v) = relay(&base, &node_id, serde_json::json!({ "session_id": " " })).await;
    assert_eq!(st.as_u16(), 400, "{v}");

    // Registered node with NO worker behind it: nothing will ever upload a
    // control result, so the relay must give up inside the (clamped) window.
    let t0 = std::time::Instant::now();
    let (st, v) = relay(
        &base,
        &node_id,
        serde_json::json!({ "session_id": "s", "timeout_ms": 250 }),
    )
    .await;
    assert_eq!(st.as_u16(), 504, "{v}");
    assert_eq!(v["timeout_ms"], 250);
    assert!(v["error"].as_str().unwrap().contains("in time"));
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(5),
        "timeout not honored: {:?}",
        t0.elapsed()
    );

    // A late upload for a control nobody waits on resolves to `false` — and
    // must still be 200 so the worker does not retry a dead delivery.
    let (st, v) = upload_result(
        &base,
        &node_id,
        &opencoder_core::node_protocol::FetchMessagesResult {
            control_id: "late-upload".into(),
            session_id: "s".into(),
            ok: true,
            error: None,
            summary: None,
            summary_seq: None,
            messages: vec![],
        },
    )
    .await;
    assert_eq!(st.as_u16(), 200, "{v}");
    assert_eq!(v["resolved"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_surfaces_worker_failure_as_502() {
    let (srv, node_id, _sid, runner, _dirs) = run_one_task().await;
    let base = srv.base.clone();
    // The worker is alive and answers, but it never saw this dialog: its
    // `ok:false` upload must reach the browser as a 502 with the reason.
    let (st, v) = relay(
        &base,
        &node_id,
        serde_json::json!({ "session_id": "sess-never-existed", "timeout_ms": 8000 }),
    )
    .await;
    assert_eq!(st.as_u16(), 502, "{v}");
    assert!(v["error"].as_str().unwrap().contains("not found"),);
    runner.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_reuses_existing_session_and_rejects_unknown() {
    let srv = spawn_server_with_store().await;
    let base = srv.base.clone();
    let node_id = register(&base, "reuse-node").await;

    // A REAL server-side session (not a synthetic node one).
    let (_, s) = post_json(&base, "/api/sessions", Some(serde_json::json!({}))).await;
    let sid = s["id"].as_str().unwrap().to_string();

    // Dispatch WITH a known session_id: the queued task must target it, and
    // the claim reply hands the SAME id back to the worker.
    let (st, d) = post_json(
        &base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": "again on this dialog", "session_id": sid })),
    )
    .await;
    assert_eq!(st.as_u16(), 200, "{d}");
    assert_eq!(
        d["session_id"],
        sid.as_str(),
        "dispatch must reuse the session"
    );

    let path = format!("/api/nodes/tasks/claim?node_id={node_id}");
    let (tsh, ts, sigh, sig) = support::sig_headers(TOKEN, "GET", &path, b"");
    let r = node_e2e_support::http()
        .get(format!("{base}{path}"))
        .header(tsh, ts)
        .header(sigh, sig)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let claim: serde_json::Value = r.json().await.unwrap();
    assert_eq!(claim["task"]["session_id"], sid.as_str());
    assert!(claim["control"].is_null(), "plain claim carries no control");

    // Unknown session_id -> 400 (typed pre-check), no task row created.
    let (st, v) = post_json(
        &base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": "x", "session_id": "sess-ghost" })),
    )
    .await;
    assert_eq!(st.as_u16(), 400, "{v}");
    let (_, tasks) = get_json(&base, &format!("/api/nodes/{node_id}/tasks")).await;
    assert_eq!(
        tasks["tasks"].as_array().unwrap().len(),
        1,
        "rejected dispatch must not queue a task"
    );
}
