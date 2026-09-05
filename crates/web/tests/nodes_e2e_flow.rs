//! Process-level e2e, full-state-machine half: a REAL worker node
//! (`opencoder_node::run_node`, mock LLM scripted) registers against a REAL
//! `build_app` server on 127.0.0.1:random-port and this test plays the
//! browser over HTTP:
//!   register → node idle → dispatch → SSE stream receives the canonical
//!   frame run llm_round_start → TextDelta×4 → llm_round_end → drain-done
//!   (empty {}) → closure-done(ok:true) → second task cancel path: 202
//!   cancelling → closure done(ok:false,cancel=true) → node idle
//!   again → task chain carries done+cancelled → durable reconciliation:
//!   the synthetic session's last session_events row is sse_kind=="done".
//!
//! The reconnect/disconnect half lives in nodes_e2e_reconnect.rs.

mod node_e2e_support;
mod support;

use std::sync::Arc;

use futures::StreamExt;
use opencoder_llm::{LlmEvent, MockChatClient};

use node_e2e_support::*;

const NODE_NAME: &str = "e2e-flow-node";

fn script(texts: &[&str]) -> Vec<LlmEvent> {
    let mut evs: Vec<LlmEvent> = texts
        .iter()
        .map(|t| LlmEvent::TextDelta(t.to_string()))
        .collect();
    evs.push(LlmEvent::Completed {
        text: texts.concat(),
        tool_calls: vec![],
        usage: None,
    });
    evs
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dispatch_stream_cancel_and_node_state_machine() {
    let srv = spawn_server().await;
    let workdir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(workdir.path());

    // Scripted round for task #1: four distinct deltas then completion.
    let texts = ["alpha ", "beta ", "gamma ", "delta"];
    let client = Arc::new(MockChatClient::new());
    let runner = spawn_node(
        &srv.base,
        NODE_NAME,
        workdir.path(),
        data.path(),
        client.clone(),
    )
    .await;

    // Browser view: the node appears once registered + first idle heartbeat.
    let node_id = wait_for(60, 100, || async {
        let (_, v) = get_json(&srv.base, "/api/nodes").await;
        v["nodes"]
            .as_array()
            .and_then(|a| a.iter().find(|n| n["name"] == *NODE_NAME))
            .filter(|n| n["status"] == "idle")
            .map(|n| n["id"].as_str().unwrap().to_string())
    })
    .await;
    assert_eq!(node_id.len(), 26, "ULID-shaped node id");

    // ── task #1: dispatch + stream the whole run ────────────────────────────
    client.queue_script(script(&texts));
    let (st1, body1) = post_json(
        &srv.base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": "flow one" })),
    )
    .await;
    assert_eq!(st1.as_u16(), 200, "{body1}");
    let tid = body1["task_id"].as_str().unwrap().to_string();
    let sid = body1["session_id"].as_str().unwrap().to_string();

    let mut sse = open_sse(&srv.base, &format!("/api/nodes/tasks/{tid}/events?after=0")).await;
    let mut seen: Vec<(String, serde_json::Value)> = Vec::new();
    while let Some(f) = sse.next().await {
        let terminal = f.kind == "done"
            && f.data.get("task_id").and_then(|t| t.as_str()) == Some(tid.as_str());
        seen.push((f.kind.clone(), f.data));
        if terminal {
            break;
        }
    }
    drop(sse);

    // Frame order mirrors the worker's canonical pipeline exactly: the LLM
    // round brackets (llm_round_start/llm_round_end) wrap the scripted deltas,
    // then drain's own empty done, then the server's closure done.
    let kinds: Vec<&str> = seen.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "llm_round_start",
            "text_delta",
            "text_delta",
            "text_delta",
            "text_delta",
            "llm_round_end",
            "done",
            "done"
        ],
        "frames={seen:?}"
    );
    for (i, want) in texts.iter().enumerate() {
        let off = i + 1; // deltas start after llm_round_start
        assert_eq!(seen[off].1["text"], *want, "text frame {i}");
    }
    assert!(
        seen[0].1.get("started_at_ms").is_some(),
        "round start stamp"
    );
    // Drain's own done is empty {}; the server's closure carries ok/task_id.
    assert_eq!(seen[6].1, serde_json::json!({}), "drain done payload");
    assert_eq!(seen[7].1["ok"], serde_json::json!(true));
    assert_eq!(seen[7].1["task_id"], tid.as_str());
    assert!(
        seen[7].1.get("cancel").is_none(),
        "success closure has no cancel flag"
    );

    // Terminal task status settles at done; the worker frees up again.
    let t_status = wait_for(60, 100, || async {
        let (_, v) = get_json(&srv.base, &format!("/api/nodes/{node_id}/tasks")).await;
        v["tasks"]
            .as_array()?
            .iter()
            .find(|t| t["id"] == tid.as_str())
            .map(|t| t["status"].as_str().unwrap_or("").to_string())
            .filter(|s| s == "done")
    })
    .await;
    assert_eq!(t_status, "done");

    // ── task #2: the cancel state machine ───────────────────────────────────
    // The hang must be queued BEFORE the claim arms it (mock contract), so
    // hold the round open until cancellation travels back via heartbeat.
    let notify = Arc::new(tokio::sync::Notify::new());
    client.queue_hang(notify.clone());
    let (st2, body2) = post_json(
        &srv.base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": "stuck job" })),
    )
    .await;
    assert_eq!(st2.as_u16(), 200, "{body2}");
    let tid2 = body2["task_id"].as_str().unwrap().to_string();

    wait_for(60, 50, || async {
        get_json(&srv.base, &format!("/api/nodes/{node_id}/tasks"))
            .await
            .1["tasks"]
            .as_array()?
            .iter()
            .find(|t| t["id"] == tid2.as_str())
            .map(|t| t["status"].as_str() == Some("running"))?
            .then_some(())
    })
    .await;

    // Attach FIRST (a browser would already be watching), then cancel.
    let mut sse2 = open_sse(
        &srv.base,
        &format!("/api/nodes/tasks/{tid2}/events?after=0"),
    )
    .await;
    let (cst, cbody) = post_json(
        &srv.base,
        &format!("/api/nodes/{node_id}/tasks/{tid2}/cancel"),
        None,
    )
    .await;
    assert_eq!(cst.as_u16(), 202, "{cbody}");
    assert_eq!(cbody["phase"], "cancelling");

    let mut closure: Option<serde_json::Value> = None;
    while let Some(f) = sse2.next().await {
        let terminal =
            (f.kind == "done" || f.kind == "error") && f.data["task_id"] == tid2.as_str();
        if terminal {
            closure = Some(f.data);
            break;
        }
    }
    let closure = closure.expect("cancel must terminate with a closure frame");
    assert_eq!(
        closure["ok"],
        serde_json::json!(false),
        "cancelled run did not complete"
    );
    assert_eq!(closure["cancel"], serde_json::json!(true));
    assert_eq!(closure["task_id"], tid2.as_str());

    // Release the hung mock stream, then give the runner a beat to settle.
    runner.abort();
    notify.notify_one();

    // Node returns to idle and both tasks coexist with their final states.
    wait_for(60, 100, || async {
        get_json(&srv.base, "/api/nodes").await.1["nodes"]
            .as_array()?
            .iter()
            .find(|n| n["id"] == node_id.as_str())
            .map(|n| n["status"] == "idle")?
            .then_some(())
    })
    .await;
    let (_, tasks) = get_json(&srv.base, &format!("/api/nodes/{node_id}/tasks")).await;
    let find = |tid: &str| {
        tasks["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == tid)
            .map(|t| t["status"].as_str().unwrap().to_string())
            .unwrap()
    };
    assert_eq!(find(&tid), "done");
    assert_eq!(find(&tid2), "cancelled");

    // Durable-side reconciliation for task #1's synthetic session.
    let rows = srv.store.events_after(&sid, -1).await.unwrap();
    let last = rows.last().expect("session has events");
    assert_eq!(last.sse_kind.as_deref(), Some("done"));
    assert_eq!(last.payload["task_id"], tid.as_str());
}
