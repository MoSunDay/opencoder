//! DAG run step query / progress API closed-loop: per-step state read from
//! the node's artifact contract (`<dag_root>/<run>/<step>/meta.json` +
//! `output.json`) through the two new control routes — the aggregate
//! progress view and the single-step view — for a finished run and for a
//! run parked mid-flight (first step done, second still pending).

use super::*;
use std::sync::Arc;

/// The DAG agent step captures its transcript from TextDelta frames, so a
/// scripted answer needs the delta plus the terminal Completed event.
fn completed(text: String) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.clone()),
        LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        },
    ]
}

fn fenced(body: &str) -> String {
    format!("result\n```json\n{body}\n```\ndone")
}

#[tokio::test]
async fn dag_run_progress_and_step_views_after_completion() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    client.queue_script(completed(fenced("{\"v\":1}")));
    client.queue_script(completed(fenced("{\"v\":2}")));
    let saved = fleet
        .call(
            "POST",
            "/api/dag/defs",
            json!({"spec":{"name":"steps-prog","steps":[
                {"name":"first","kind":{"type":"agent","prompt":"produce one"}},
                {"name":"second","depends_on":["first"],"kind":{"type":"agent","prompt":"produce two"}}
            ]}}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let dispatched = fleet
        .call(
            "POST",
            "/api/dag/defs/steps-prog/dispatch",
            json!({"id":"dag-steps-prog-run"}),
        )
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");
    let detail = settled(&fleet.nodes[0], "dag-steps-prog-run").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");

    // Progress view: spec-ordered steps + counts folded from meta outcomes.
    let progress = fleet
        .call(
            "GET",
            "/api/dag/runs/dag-steps-prog-run/progress",
            Value::Null,
        )
        .await;
    assert_eq!(progress.status, 200, "{progress:?}");
    assert_eq!(progress.body["run_id"], json!("dag-steps-prog-run"));
    assert_eq!(progress.body["execution_status"], json!("done"));
    assert_eq!(progress.body["total"], json!(2));
    assert_eq!(progress.body["done"], json!(2));
    assert_eq!(progress.body["error"], json!(0));
    assert_eq!(progress.body["cancelled"], json!(0));
    assert_eq!(progress.body["pending"], json!(0));
    let steps = progress.body["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert_eq!(steps[0]["name"], json!("first"));
    assert_eq!(steps[0]["status"], json!("done"));
    assert_eq!(steps[0]["error"], json!(null));
    assert_eq!(steps[1]["name"], json!("second"));
    assert_eq!(steps[1]["status"], json!("done"));

    // Single-step view: bounded structured output + real timings.
    let first = fleet
        .call("GET", "/api/dag/runs/dag-steps-prog-run/steps/first", Value::Null)
        .await;
    assert_eq!(first.status, 200, "{first:?}");
    assert_eq!(first.body["name"], json!("first"));
    assert_eq!(first.body["status"], json!("done"));
    assert_eq!(first.body["error"], json!(null));
    assert_eq!(first.body["output"], json!({"v":1}));
    assert!(first.body["started_at_ms"].is_i64(), "{first:?}");
    assert!(first.body["finished_at_ms"].is_i64(), "{first:?}");
    let second = fleet
        .call(
            "GET",
            "/api/dag/runs/dag-steps-prog-run/steps/second",
            Value::Null,
        )
        .await;
    assert_eq!(second.status, 200, "{second:?}");
    assert_eq!(second.body["output"], json!({"v":2}));

    // Unknown step (not in the spec) is a 404, not a pending row.
    let missing = fleet
        .call(
            "GET",
            "/api/dag/runs/dag-steps-prog-run/steps/missing",
            Value::Null,
        )
        .await;
    assert_eq!(missing.status, 404, "{missing:?}");
    fleet.shutdown().await;
}

#[tokio::test]
async fn dag_run_progress_reports_pending_step_while_in_flight() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    client.queue_script(completed(fenced("{\"v\":1}")));
    // Park the second step's LLM call so the run stays `running` with one
    // step done and one not yet folded into the artifact contract.
    client.queue_hang(Arc::new(tokio::sync::Notify::new()));
    let saved = fleet
        .call(
            "POST",
            "/api/dag/defs",
            json!({"spec":{"name":"steps-mid","steps":[
                {"name":"first","kind":{"type":"agent","prompt":"produce one"}},
                {"name":"second","depends_on":["first"],"kind":{"type":"agent","prompt":"produce two"}}
            ]}}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let dispatched = fleet
        .call(
            "POST",
            "/api/dag/defs/steps-mid/dispatch",
            json!({"id":"dag-steps-mid-run"}),
        )
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");

    let progress = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let progress = fleet
                .call(
                    "GET",
                    "/api/dag/runs/dag-steps-mid-run/progress",
                    Value::Null,
                )
                .await;
            assert_eq!(progress.status, 200, "{progress:?}");
            if progress.body["done"] == json!(1) && progress.body["pending"] == json!(1) {
                return progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("run never reached done=1 pending=1");
    assert_eq!(progress.body["total"], json!(2), "{progress:?}");
    assert_eq!(progress.body["execution_status"], json!("running"));
    assert_eq!(progress.body["done"], json!(1));
    assert_eq!(progress.body["error"], json!(0));
    assert_eq!(progress.body["cancelled"], json!(0));
    assert_eq!(progress.body["pending"], json!(1));
    let steps = progress.body["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert_eq!(steps[0]["name"], json!("first"));
    assert_eq!(steps[0]["status"], json!("done"));
    assert_eq!(steps[1]["name"], json!("second"));
    assert_eq!(steps[1]["status"], json!("pending"));

    // In-spec step that never ran: 200 with pending status and null
    // timings/output (no meta.json, no output.json on disk yet).
    let second = fleet
        .call(
            "GET",
            "/api/dag/runs/dag-steps-mid-run/steps/second",
            Value::Null,
        )
        .await;
    assert_eq!(second.status, 200, "{second:?}");
    assert_eq!(second.body["status"], json!("pending"));
    assert_eq!(second.body["output"], json!(null));
    assert_eq!(second.body["started_at_ms"], json!(null));
    assert_eq!(second.body["finished_at_ms"], json!(null));

    // Wind the parked run down so the fleet can shut down cleanly.
    let cancelled = fleet
        .call(
            "POST",
            "/api/dag/runs/dag-steps-mid-run/cancel",
            Value::Null,
        )
        .await;
    assert_eq!(cancelled.status, 200, "{cancelled:?}");
    let detail = settled(&fleet.nodes[0], "dag-steps-mid-run").await;
    assert_eq!(detail["execution"]["status"], "cancelled", "{detail}");
    fleet.shutdown().await;
}
