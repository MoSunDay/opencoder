//! DAG contracts, part 2 (runs), ported from `tests/dag_store.rs`: the
//! status grid, atomic finalize with its synthetic `run_finished` frame,
//! the cancel piggyback, event cursoring and the lost-node sweep.

use super::*;
use opencoder_dag::DagRunStatus;
use opencoder_store::{ConvergedDagRun, DagEventRecord};
use serde_json::json;

// ── status transitions / cancel ───────────────────────────────────────────

#[tokio::test]
async fn status_transitions_stamp_finished_and_freeze_terminal() {
    let store = mem().await;
    let node = register(&store, "n1", 1_000).await;
    dispatch_dag(&store, "r-1", None, 1_500).await;
    store.claim_next_dag_run(&node.id, 1_600).await.unwrap();

    let done = store
        .update_dag_run_status("r-1", DagRunStatus::Done, None, 2_000)
        .await
        .unwrap();
    assert_eq!(done.status, DagRunStatus::Done);
    assert_eq!(done.error, None);
    assert_eq!(
        done.finished_at,
        Some(2_000),
        "terminal write stamps finished_at"
    );

    // Terminal freeze: every move out of done bails, same-state no-ops too.
    assert!(store
        .update_dag_run_status("r-1", DagRunStatus::Running, None, 2_100)
        .await
        .is_err());
    assert!(store
        .update_dag_run_status("r-1", DagRunStatus::Done, None, 2_100)
        .await
        .is_err());

    // running -> error stores the error and stamps finished_at.
    dispatch_dag(&store, "r-2", None, 2_200).await;
    store.claim_next_dag_run(&node.id, 2_300).await.unwrap();
    let errored = store
        .update_dag_run_status("r-2", DagRunStatus::Error, Some("boom"), 2_400)
        .await
        .unwrap();
    assert_eq!(errored.status, DagRunStatus::Error);
    assert_eq!(errored.error.as_deref(), Some("boom"));
    assert_eq!(errored.finished_at, Some(2_400));

    // Unknown ids bail with a clear message.
    let err = store
        .update_dag_run_status("ghost", DagRunStatus::Running, None, 2_500)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "got: {err}");
}

/// `finalize_dag_run` lands the terminal status AND its synthetic
/// `run_finished` frame in ONE transaction: the returned seq points at a
/// durable frame row, re-finalizing a terminal run bails "illegal" and
/// unknown ids bail "not found" without appending anything.
#[tokio::test]
async fn finalize_commits_status_and_frame_atomically() {
    let store = mem().await;
    let node = register(&store, "n1", 1_000).await;
    dispatch_dag(&store, "r-ok", None, 1_500).await;
    store.claim_next_dag_run(&node.id, 1_600).await.unwrap();

    let seq = store
        .finalize_dag_run("r-ok", DagRunStatus::Done, None, 2_000)
        .await
        .unwrap();
    assert!(seq > 0);

    let done = store.get_dag_run("r-ok").await.unwrap().unwrap();
    assert_eq!(done.status, DagRunStatus::Done);
    assert_eq!(done.finished_at, Some(2_000));

    let frames = store.dag_events_after("r-ok", 0, 10).await.unwrap();
    assert_eq!(frames.len(), 1, "exactly one synthetic frame: {frames:?}");
    assert_eq!(frames[0].kind, "run_finished");
    assert_eq!(frames[0].seq, Some(seq));
    assert_eq!(frames[0].step, None);
    assert_eq!(frames[0].payload["status"], "done");
    assert!(frames[0].payload.get("error").is_none());
    assert_eq!(frames[0].at_ms, 2_000);

    // Terminal freeze: re-finalizing bails "illegal".
    let err = store
        .finalize_dag_run("r-ok", DagRunStatus::Error, Some("late"), 2_100)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("illegal"), "got: {err}");

    // Unknown ids bail "not found" and append nothing.
    let err = store
        .finalize_dag_run("ghost", DagRunStatus::Done, None, 2_200)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "got: {err}");
    assert_eq!(
        store.dag_events_after("r-ok", 0, 10).await.unwrap().len(),
        1,
        "failed finalizes append no frames"
    );

    // Error reports carry their text through to the frame payload.
    dispatch_dag(&store, "r-err", None, 2_300).await;
    store.claim_next_dag_run(&node.id, 2_400).await.unwrap();
    let seq_err = store
        .finalize_dag_run("r-err", DagRunStatus::Error, Some("boom"), 2_500)
        .await
        .unwrap();
    let err_frames = store.dag_events_after("r-err", 0, 10).await.unwrap();
    assert_eq!(err_frames.len(), 1);
    assert_eq!(err_frames[0].seq, Some(seq_err));
    assert_eq!(err_frames[0].payload["status"], "error");
    assert_eq!(err_frames[0].payload["error"], "boom");
}

/// `pending -> cancelled` collapses directly; `running -> cancelling`
/// surfaces through `cancelling_dag_runs` (heartbeat piggyback); repeats
/// are idempotent; terminal bails; unknown bails.
#[tokio::test]
async fn cancel_pending_direct_and_running_via_cancelling() {
    let store = mem().await;
    let node = register(&store, "n1", 1_000).await;

    dispatch_dag(&store, "r-pend", None, 1_500).await;
    store.cancel_dag_run("r-pend", 1_600).await.unwrap();
    let cancelled = store.get_dag_run("r-pend").await.unwrap().unwrap();
    assert_eq!(cancelled.status, DagRunStatus::Cancelled);
    assert_eq!(cancelled.finished_at, Some(1_600));
    assert!(store.cancel_dag_run("r-pend", 1_700).await.is_err());

    dispatch_dag(&store, "r-run", None, 1_800).await;
    store.claim_next_dag_run(&node.id, 1_900).await.unwrap();
    store.cancel_dag_run("r-run", 2_000).await.unwrap();
    let cancelling = store.get_dag_run("r-run").await.unwrap().unwrap();
    assert_eq!(cancelling.status, DagRunStatus::Cancelling);
    assert_eq!(cancelling.finished_at, None, "cancelling is not terminal");
    assert_eq!(
        store.cancelling_dag_runs(&node.id).await.unwrap(),
        ["r-run"]
    );

    store.cancel_dag_run("r-run", 2_100).await.unwrap(); // idempotent repeat
    assert!(store.cancel_dag_run("ghost", 2_100).await.is_err());
}

// ── events ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn events_assign_ascending_seqs_and_paginate_by_cursor() {
    let store = mem().await;
    let events: Vec<DagEventRecord> = (0..5)
        .map(|i| DagEventRecord {
            seq: None,
            run_id: "r-1".into(),
            kind: "step_done".into(),
            step: Some(format!("s{i}")),
            payload: json!({"i": i}),
            at_ms: 1_000 + i,
        })
        .collect();
    let seqs = store.append_dag_events(&events).await.unwrap();
    assert_eq!(seqs.len(), 5);
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seqs ascend: {seqs:?}"
    );

    let all = store.dag_events_after("r-1", 0, 100).await.unwrap();
    let steps: Vec<&str> = all.iter().map(|e| e.step.as_deref().unwrap()).collect();
    assert_eq!(steps, ["s0", "s1", "s2", "s3", "s4"]);
    assert!(all.iter().all(|e| e.seq.is_some()));
    assert_eq!(all[2].payload, json!({"i": 2}), "payload round-trips");

    // Cursor: after seqs[2] returns exactly the tail; limit is respected.
    let tail = store.dag_events_after("r-1", seqs[2], 100).await.unwrap();
    let tail_steps: Vec<&str> = tail.iter().map(|e| e.step.as_deref().unwrap()).collect();
    assert_eq!(tail_steps, ["s3", "s4"]);
    assert_eq!(store.dag_events_after("r-1", 0, 2).await.unwrap().len(), 2);
    assert!(store
        .dag_events_after("other", 0, 100)
        .await
        .unwrap()
        .is_empty());
    assert!(store.append_dag_events(&[]).await.unwrap().is_empty());
}

// ── lost-node sweep ───────────────────────────────────────────────────────

/// Heartbeat-stale nodes have their `running` AND `cancelling` runs
/// collapsed to `error("node lost")` — each with its `run_finished` frame
/// in the same batch; fresh nodes, pending rows and terminal rows stay put;
/// a re-run is a no-op.
#[tokio::test]
async fn lost_sweep_converges_running_and_cancelling() {
    const STALE_MS: i64 = 400;
    let store = mem().await;
    let stale_a = register(&store, "stale-a", 1_000).await;
    let stale_b = register(&store, "stale-b", 1_000).await;
    let alive = register(&store, "alive-node", 1_301).await; // gap 100 < 400

    // Stale node A: one claimed run asked to cancel (now `cancelling`).
    dispatch_dag(&store, "r-can", Some(&stale_a.id), 1_100).await;
    store.claim_next_dag_run(&stale_a.id, 1_150).await.unwrap();
    store.cancel_dag_run("r-can", 1_200).await.unwrap();
    // Stale node B: one still-`running` run — a SEPARATE node, because the
    // fixed busy guard keeps a cancelling node from claiming anything new.
    dispatch_dag(&store, "r-run", Some(&stale_b.id), 1_210).await;
    store.claim_next_dag_run(&stale_b.id, 1_250).await.unwrap();
    // Fresh node keeps its running run; an unpinned pending row waits.
    dispatch_dag(&store, "r-live", Some(&alive.id), 1_100).await;
    store.claim_next_dag_run(&alive.id, 1_150).await.unwrap();
    dispatch_dag(&store, "r-pend", None, 1_300).await;

    let converged = store.converge_lost_dag_runs(1_401, STALE_MS).await.unwrap();
    let mut ids: Vec<&str> = converged.iter().map(|c| c.record.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["r-can", "r-run"]);
    for c in &converged {
        assert_frame_landed(&store, c).await;
    }
    let live = store.get_dag_run("r-live").await.unwrap().unwrap();
    assert_eq!(live.status, DagRunStatus::Running);
    assert_eq!(live.finished_at, None);
    let pending = store.get_dag_run("r-pend").await.unwrap().unwrap();
    assert_eq!(pending.status, DagRunStatus::Pending);

    // Idempotent: terminal rows are frozen, so the re-sweep finds nothing.
    assert!(store
        .converge_lost_dag_runs(1_500, STALE_MS)
        .await
        .unwrap()
        .is_empty());
}

async fn assert_frame_landed(store: &RdbStore, c: &ConvergedDagRun) {
    assert_eq!(c.record.status, DagRunStatus::Error);
    assert_eq!(c.record.error.as_deref(), Some("node lost"));
    assert_eq!(c.record.finished_at, Some(1_401));
    // The synthetic final frame landed in the SAME batch as the flip: the
    // returned seq points at a durable `run_finished` row.
    assert!(c.run_finished_seq > 0, "seq must be assigned");
    let frames = store.dag_events_after(&c.record.id, 0, 10).await.unwrap();
    let frame = frames
        .iter()
        .find(|f| f.kind == "run_finished")
        .expect("run_finished frame");
    assert_eq!(frame.seq, Some(c.run_finished_seq));
    assert_eq!(frame.payload["status"], "error");
    assert_eq!(frame.payload["error"], "node lost");
}
