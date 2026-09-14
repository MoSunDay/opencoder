//! Paging, cursor and termination behaviour: filtered-page scan bounds, the
//! 200-frame page cap, the terminal `step_finished` frame and validation.

use super::*;
use opencoder_core::fleet::{ExecutionKind, ExecutionRef, ExecutionStatus};
use serde_json::{json, Value};

#[tokio::test]
async fn dag_step_events_scans_past_a_fully_filtered_page() {
    let (_root, worker) = worker().await;
    let run = "dag-step-scan";
    dag_run(
        &worker,
        run,
        vec![step_spec("noise", "wasm"), step_spec("fetch", "wasm")],
    )
    .await;
    session(&worker, run).await;
    // One full page of another step's output, then this step's single frame.
    let mut rows: Vec<(&str, Value)> = Vec::new();
    for _ in 0..200 {
        rows.push(("step_output", json!({"step":"noise","text":"x"})));
    }
    rows.push(("step_output", json!({"step":"fetch","text":"mine"})));
    let seqs = append(&worker, run, rows).await;

    let reply = dag_step_events(&worker, &dag_ref(run), "fetch", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    // The filtered-out page still advanced the scan cursor, so the step's own
    // frame is found and `more` reports the real tail state.
    assert_eq!(frames(&reply).len(), 1, "{reply:?}");
    assert_eq!(frames(&reply)[0]["seq"], json!(seqs[200]));
    assert_eq!(frames(&reply)[0]["data"]["text"], json!("mine"));
    assert_eq!(reply.body["more"], json!(false));
    assert_eq!(reply.body["finished"], json!(false));
    assert_eq!(reply.body["head_seq"], json!(201));

    let tail = dag_step_events(&worker, &dag_ref(run), "fetch", seqs[200])
        .await
        .unwrap();
    assert!(frames(&tail).is_empty(), "{tail:?}");
    assert_eq!(tail.body["more"], json!(false));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_caps_the_filter_scan_and_reports_more() {
    let (_root, worker) = worker().await;
    let run = "dag-step-cap";
    dag_run(
        &worker,
        run,
        vec![step_spec("noise", "wasm"), step_spec("fetch", "wasm")],
    )
    .await;
    session(&worker, run).await;
    // More than MAX_FILTER_PAGES * EVENT_PAGE_MAX frames, none of them ours:
    // one poll must not replay the whole run session.
    let rows: Vec<(&str, Value)> = (0..1650)
        .map(|_| ("step_output", json!({"step":"noise","text":"x"})))
        .collect();
    append(&worker, run, rows).await;

    let reply = dag_step_events(&worker, &dag_ref(run), "fetch", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert!(frames(&reply).is_empty(), "{reply:?}");
    assert_eq!(reply.body["more"], json!(true));
    assert_eq!(reply.body["finished"], json!(false));
    assert_eq!(reply.body["head_seq"], json!(1650));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_terminal_step_appends_step_finished() {
    let (_root, worker) = worker().await;
    let run = "dag-step-terminal";
    dag_run(
        &worker,
        run,
        vec![step_spec("build", "wasm"), step_spec("noise", "wasm")],
    )
    .await;
    session(&worker, run).await;
    artifacts(
        &worker,
        run,
        "build",
        &[(
            "meta.json",
            json!({
                "outcome": "error",
                "error": "exit 3: boom",
                "started_at_ms": 5,
                "finished_at_ms": 9,
            }),
        )],
    );
    let seqs = append(
        &worker,
        run,
        vec![
            (
                "step_output",
                json!({"step":"build","stream":"stdout","text":"working"}),
            ),
            ("step_output", json!({"step":"noise","text":"other"})),
            (
                "step_output",
                json!({"step":"build","stream":"stderr","text":"last"}),
            ),
        ],
    )
    .await;

    let reply = dag_step_events(&worker, &dag_ref(run), "build", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(frames(&reply).len(), 3, "{reply:?}");
    let last = frames(&reply).last().unwrap();
    assert_eq!(last["seq"], json!(seqs[2] + 1));
    assert_eq!(last["kind"], json!("step_finished"));
    assert_eq!(
        last["data"],
        json!({
            "status": "error",
            "error": "exit 3: boom",
            "started_at_ms": 5,
            "finished_at_ms": 9,
        })
    );
    assert!(last["ts"].is_i64());
    assert_eq!(reply.body["finished"], json!(true));
    assert_eq!(reply.body["more"], json!(false));
    assert_eq!(reply.body["step"]["status"], json!("error"));
    assert_eq!(reply.body["step"]["error"], json!("exit 3: boom"));
    assert_eq!(reply.body["step"]["started_at_ms"], json!(5));
    assert_eq!(reply.body["step"]["finished_at_ms"], json!(9));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_withholds_step_finished_until_the_last_page() {
    let (_root, worker) = worker().await;
    let run = "dag-step-paged";
    let child = "01HZDAGSTEPCHILD0000000002";
    dag_run(&worker, run, vec![step_spec("plan", "agent")]).await;
    session(&worker, run).await;
    session(&worker, child).await;
    artifacts(
        &worker,
        run,
        "plan",
        &[
            ("session.json", json!({"session_id": child})),
            (
                "meta.json",
                json!({"outcome": "done", "started_at_ms": 2, "finished_at_ms": 7}),
            ),
        ],
    );
    let rows: Vec<(&str, Value)> = (0..250)
        .map(|index| ("message", json!({"text": index})))
        .collect();
    let seqs = append(&worker, child, rows).await;

    // First page: a full 200-frame page, so `more` is set and the terminal
    // frame is withheld until the client has seen the last frame.
    let first = dag_step_events(&worker, &dag_ref(run), "plan", 0)
        .await
        .unwrap();
    assert_eq!(first.status, 200, "{first:?}");
    assert_eq!(frames(&first).len(), 200, "{first:?}");
    assert_eq!(first.body["more"], json!(true));
    assert_eq!(first.body["finished"], json!(false));
    assert_eq!(frames(&first).last().unwrap()["seq"], json!(seqs[199]));

    let second = dag_step_events(&worker, &dag_ref(run), "plan", seqs[199])
        .await
        .unwrap();
    assert_eq!(frames(&second).len(), 51, "{second:?}");
    assert_eq!(second.body["more"], json!(false));
    assert_eq!(second.body["finished"], json!(true));
    assert_eq!(
        frames(&second).last().unwrap()["kind"],
        json!("step_finished")
    );
    assert_eq!(
        frames(&second).last().unwrap()["data"]["status"],
        json!("done")
    );
    assert_eq!(frames(&second).last().unwrap()["seq"], json!(seqs[249] + 1));
    assert_eq!(second.body["step"]["status"], json!("done"));
    assert_eq!(second.body["step"]["session_id"], json!(child));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_rejects_unknown_runs_steps_and_kinds() {
    let (_root, worker) = worker().await;
    let run = "dag-step-errors";
    dag_run(&worker, run, vec![step_spec("fetch", "wasm")]).await;

    let unknown_step = dag_step_events(&worker, &dag_ref(run), "nope", 0)
        .await
        .unwrap();
    assert_eq!(unknown_step.status, 404, "{unknown_step:?}");
    assert_eq!(
        unknown_step.body["error"],
        json!("step not found in run spec")
    );

    let unknown_run = dag_step_events(&worker, &dag_ref("no-such-run"), "fetch", 0)
        .await
        .unwrap();
    assert_eq!(unknown_run.status, 404, "{unknown_run:?}");

    // A non-DAG execution has no step stream at all.
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record(&worker, "todos-run", ExecutionStatus::Running))
        .unwrap();
    let wrong_kind = dag_step_events(
        &worker,
        &ExecutionRef {
            id: "todos-run".into(),
            kind: ExecutionKind::Todos,
        },
        "fetch",
        0,
    )
    .await
    .unwrap();
    assert_eq!(wrong_kind.status, 400, "{wrong_kind:?}");
    worker.shutdown().await.unwrap();
}
