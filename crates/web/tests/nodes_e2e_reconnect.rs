//! Process-level e2e, reconnect half: proves the `?after=<seq>` replay seam
//! against a REAL server + REAL worker node. A scripted task streams deltas
//! over a live SSE connection; the "browser" drops the connection after the
//! second delta, then reconnects with `after` pinned to the cutoff row's seq
//! (read from the durable store). Assertions:
//!   • no loss:  first-half ∪ resumed == full canonical frame run
//!   • no dup:   resumed frames carry no seq ≤ cutoff and never repeat content
//!   • reconcil: store rows for the synthetic session ≡ frames both passes saw
//!   • closure:  the resumed stream terminates at done(ok:true, task_id)
//!
//! The full-state-machine half lives in nodes_e2e_flow.rs.

mod node_e2e_support;

use std::sync::Arc;

use futures::StreamExt;
use opencoder_llm::{LlmEvent, MockChatClient};

use node_e2e_support::*;

const NODE_NAME: &str = "e2e-reconnect-node";

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
async fn mid_stream_disconnect_resumes_without_loss_or_duplication() {
    let srv = spawn_server().await;
    let workdir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(workdir.path());

    let client = Arc::new(MockChatClient::new());
    let _runner = spawn_node(
        &srv.base,
        NODE_NAME,
        workdir.path(),
        data.path(),
        client.clone(),
    )
    .await;

    let node_id = wait_for(15, 100, || async {
        let (_, v) = get_json(&srv.base, "/api/nodes").await;
        v["nodes"]
            .as_array()
            .and_then(|a| a.iter().find(|n| n["name"] == *NODE_NAME))
            .filter(|n| n["status"] == "idle")
            .map(|n| n["id"].as_str().unwrap().to_string())
    })
    .await;

    let texts = ["one ", "two ", "three ", "four"];
    client.queue_script(script(&texts));
    let (st, body) = post_json(
        &srv.base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": "reconnect probe" })),
    )
    .await;
    assert_eq!(st.as_u16(), 200, "{body}");
    let tid = body["task_id"].as_str().unwrap().to_string();
    let sid = body["session_id"].as_str().unwrap().to_string();

    // ── pass 1: live tail, drop after the 2nd delta ─────────────────────────
    let mut sse1 = open_sse(&srv.base, &format!("/api/nodes/tasks/{tid}/events?after=0")).await;
    let mut first_pass: Vec<(String, serde_json::Value)> = Vec::new();
    while let Some(f) = sse1.next().await {
        let is_delta = f.kind == "text_delta";
        first_pass.push((f.kind.clone(), f.data.clone()));
        if is_delta && first_pass.iter().filter(|(k, _)| k == "text_delta").count() >= 2 {
            break;
        }
    }
    drop(sse1);
    let got_deltas: Vec<&str> = first_pass
        .iter()
        .filter(|(k, _)| k == "text_delta")
        .map(|(_, d)| d["text"].as_str().unwrap())
        .collect();
    assert_eq!(got_deltas, &texts[..2], "pass-1 delta prefix");

    // Cutoff = durable seq of the 2nd persisted text_delta row (persist-before-
    // broadcast ⇒ wire order follows persistence order).
    let rows1 = srv.store.events_after(&sid, -1).await.unwrap();
    let cutoff = rows1
        .iter()
        .filter(|r| r.sse_kind.as_deref() == Some("text_delta"))
        .nth(1)
        .and_then(|r| r.seq)
        .expect("2nd persisted text_delta row");

    // ── pass 2: reconnect strictly AFTER the cutoff ─────────────────────────
    let mut sse2 = open_sse(
        &srv.base,
        &format!("/api/nodes/tasks/{tid}/events?after={cutoff}"),
    )
    .await;
    let mut resumed: Vec<(String, serde_json::Value)> = Vec::new();
    loop {
        let next = tokio::time::timeout(std::time::Duration::from_secs(10), sse2.next()).await;
        match next {
            Ok(Some(f)) => {
                let terminal = (f.kind == "done" || f.kind == "error")
                    && f.data.get("task_id").and_then(|t| t.as_str()) == Some(tid.as_str());
                resumed.push((f.kind.clone(), f.data));
                if terminal {
                    break;
                }
            }
            _ => panic!("resumed stream stalled without reaching a closure frame"),
        }
    }
    drop(sse2);

    // Closure frame ends the resumed stream.
    let closure = &resumed.last().unwrap().1;
    assert_eq!(closure["ok"], serde_json::json!(true), "closure={closure}");
    assert_eq!(closure["task_id"], tid.as_str());

    // No dup across the seam: resumed rows are exactly what durability kept
    // past `cutoff`, nothing replayed twice.
    let mut final_rows = srv.store.events_after(&sid, -1).await.unwrap();
    final_rows.sort_by_key(|r| r.seq.unwrap_or(i64::MAX));
    let expected_tail: Vec<(String, serde_json::Value)> = final_rows
        .iter()
        .filter(|r| r.seq.is_some_and(|s| s > cutoff))
        .map(|r| {
            (
                r.sse_kind
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", r.kind)),
                r.payload.clone(),
            )
        })
        .collect();
    assert_eq!(
        resumed, expected_tail,
        "resumed window must equal durable tail after seq {cutoff}"
    );

    // Reconcile totals: one first_pass + one resumed each store row, no loss.
    let total_frames = first_pass.len() + resumed.len();
    assert_eq!(
        total_frames,
        final_rows.len(),
        "frames seen on the wire must reconcile with durable session_events"
    );
    // And full concatenation preserves canonical order incl. all deltas once.
    let all: Vec<&str> = first_pass
        .iter()
        .chain(resumed.iter())
        .filter(|(k, _)| k == "text_delta")
        .map(|(_, d)| d["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        all,
        texts.to_vec(),
        "deltas appear exactly once, in script order"
    );

    // Task ledger settled.
    wait_for(15, 100, || async {
        get_json(&srv.base, &format!("/api/nodes/{node_id}/tasks"))
            .await
            .1["tasks"]
            .as_array()?
            .iter()
            .find(|t| t["id"] == tid.as_str())
            .map(|t| t["status"].as_str() == Some("done"))?
            .then_some(())
    })
    .await;
}
