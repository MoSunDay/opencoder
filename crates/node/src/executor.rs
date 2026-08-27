//! Task executor: run one claimed node task to completion on this machine.
//!
//! A thin adapter over the `opencoder-session` public primitives — the same
//! building blocks `crates/web/src/handle.rs` composes for its drain tasks:
//! `resume_and_replay` (build/resume the session against the local store),
//! hard-cancel via `session.cancel` (the web `/stop` token), and
//! `spawn_event_flusher` (durable local event persistence). The drain loop
//! itself is NEVER copied: [`opencoder_session::run`] owns it; this module
//! only supplies the prompt and an event callback.
//!
//! Events flow two ways: locally through the flusher (replay parity with a
//! web-driven session) and remotely as ordered [`NodeEventIn`] batches
//! uploaded via [`Uplink`] with bounded retry. The wire mapping reuses
//! `SessionEvent::sse_kind()` / `sse_data()` — the session crate's own single
//! source of truth, shared with the web layer's `sse_from_session_event` — so
//! a node's uploaded stream replays identically on either surface.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_core::node_protocol::{ClaimedTask, NodeEventBatch, NodeEventIn};
use opencoder_llm::ChatStream;
use opencoder_session::{resume_and_replay as resume_session, run as run_session, SessionEvent};
use opencoder_store::{SessionMeta, Store};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::batcher::{wire_event, Batcher};
use crate::uplink::Uplink;

/// Everything execution needs that does not change per task.
pub struct ExecDeps {
    pub store: Arc<dyn Store>,
    pub client: Arc<dyn ChatStream>,
    pub workdir: std::path::PathBuf,
    pub config: opencoder_core::Config,
}

/// Map one session event onto its wire shape (identical data to what the web
/// layer persists, flattened into kind + payload + emitter clock).
fn to_wire(ev: &SessionEvent) -> NodeEventIn {
    wire_event(ev.sse_kind(), ev.sse_data())
}

/// Run `task` end-to-end: persist the remote-issued ids locally, execute one
/// full drain, stream events upstream in batches, then report a terminal
/// status (`done | error | cancelled`). Upload failures are retried a bounded
/// number of times and finally demoted to warnings — a flaky uplink never
/// breaks a healthy local run.
///
/// `cancel_rx` carries the heartbeater-fed cancel flag (`true` = the server
/// asked to abort); once observed, the hard-cancel token fires and execution
/// converges through the runner's own interrupt path before reporting
/// `cancelled`.
pub async fn execute(
    uplink: &Uplink,
    deps: ExecDeps,
    task: &ClaimedTask,
    cancel_rx: watch::Receiver<bool>,
) -> Result<()> {
    create_local_meta(&deps.store, &deps.workdir, task).await?;
    // One token doubles as replay guard AND run-loop hard cancel (web parity:
    // start_drain_locked passes a single token to both call sites).
    let cancel = CancellationToken::new();
    let mut session = resume_session(
        deps.store.clone(),
        &task.session_id,
        deps.config.clone(),
        deps.client.clone(),
        deps.workdir.clone(),
        Some(cancel.clone()),
    )
    .await?;
    session.cancel = Some(cancel.clone());
    // Fresh per-task turn token so an interrupt never leaks into later tasks.
    session.turn_cancel = Some(Arc::new(std::sync::Mutex::new(CancellationToken::new())));

    // Local durability of the event stream (drain-command and turn events
    // land in the local DB exactly like a web-driven session would). The
    // sink is NOT Clone: it moves into the event callback and is dropped
    // when the run finishes, closing the channel for the tail flush below.
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(deps.store.clone()), task.session_id.clone());

    // Batched remote upload pipeline: the sync event callback cannot await,
    // so it only enqueues complete batches; a dedicated uploader consumes them
    // IN ORDER (unbounded channel preserves batch order) with bounded retry.
    let (batch_tx, mut batch_rx) = tokio::sync::mpsc::unbounded_channel::<NodeEventBatch>();
    let batcher = Arc::new(Mutex::new(Batcher::new()));
    let on_event = {
        let batcher = Arc::clone(&batcher);
        let tx = batch_tx.clone();
        move |ev: SessionEvent| {
            let _ = sink.push(&ev);
            let mut b = lock_batcher(&batcher);
            b.push(to_wire(&ev));
            if b.should_flush() {
                let events = b.take();
                let _ = tx.send(NodeEventBatch { events });
            }
        }
    };

    info!(
        task_id = %task.task_id,
        session_id = %task.session_id,
        "node task executing"
    );
    let result = run_with_cancel(
        &mut session,
        task.prompt.clone(),
        cancel.clone(),
        cancel_rx,
        on_event,
    )
    .await;

    // Final partial batch, then stop the uploader; guarantee the tail flush
    // of local persistence BEFORE reporting a terminal state.
    let tail = lock_batcher(&batcher).take();
    if !tail.is_empty() {
        let _ = batch_tx.send(NodeEventBatch { events: tail });
    }
    drop(batch_tx);
    // `sink` already dropped with the consumed closure: the channel is
    // closed, so awaiting the flusher guarantees the final local flush.
    if let Err(e) = flusher.await {
        warn!(task_id = %task.task_id, error = %e, "local event flush failed");
    }
    uploader(uplink, &task.task_id, &mut batch_rx).await;

    let report = terminal_report(cancel.is_cancelled(), result.as_ref().err());
    if let Err(e) = uplink
        .report_status(&task.task_id, report.0, report.1.clone())
        .await
    {
        warn!(task_id = %task.task_id, status = report.0, error = %e, "status report failed");
    }
    match (&result, report.0) {
        (Err(e), "error") => Err(anyhow::anyhow!("node task {}: {e:#}", task.task_id)),
        _ => Ok(()),
    }
}

/// Await the plain full-run future while racing the server-side cancel flag.
///
/// The loop form keeps `fut` pinned across iterations: when the flag flips we
/// fire the hard-cancel token and KEEP draining `fut`, so the session closes
/// through its own interrupt path (tool-result bookkeeping, tail messages)
/// instead of being dropped mid-flight. If the run finished first, the `fut`
/// arm simply wins on the next poll.
async fn run_with_cancel(
    session: &mut opencoder_session::SessionState,
    prompt: String,
    cancel: CancellationToken,
    mut flag_rx: watch::Receiver<bool>,
    on_event: impl FnMut(SessionEvent) + Send,
) -> Result<()> {
    let fut = run_session(session, prompt, on_event);
    tokio::pin!(fut);
    // One-shot guard: after firing, stop selecting the flag arm entirely.
    // Without it an already-true flag re-resolves instantly on every biased
    // poll and starves `fut` (livelock: cancel fires forever, run never ends).
    let mut fired = false;
    loop {
        tokio::select! {
            biased;
            _ = await_flag(&mut flag_rx), if !fired => {
                warn!("server requested cancellation; interrupting local turn");
                cancel.cancel();
                // Keep draining `fut`: the session converges through its own
                // interrupt path and still emits the tail events we upload.
                fired = true;
            }
            res = &mut fut => return res,
        }
    }
}

/// Resolve once the watched flag turns `true`. A dropped sender (the task's
/// heartbeater gone) parks forever rather than synthesizing a cancel.
async fn await_flag(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Ordered uploader: drains every queued batch; each upload retries up to
/// three times with linear backoff, then logs-and-drops (warn-only).
async fn uploader(
    uplink: &Uplink,
    tid: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<NodeEventBatch>,
) {
    const ATTEMPTS: usize = 3;
    while let Some(batch) = rx.recv().await {
        for attempt in 0..ATTEMPTS {
            match uplink.upload_events(tid, batch.clone()).await {
                Ok(()) => break,
                Err(e) => {
                    if attempt + 1 == ATTEMPTS {
                        warn!(
                            task_id = %tid,
                            events = batch.events.len(),
                            error = %e,
                            "event upload failed after retries; dropping batch"
                        );
                    } else {
                        warn!(task_id = %tid, attempt, error = %e, "event upload retrying");
                        tokio::time::sleep(std::time::Duration::from_millis(
                            200 * (attempt as u64 + 1),
                        ))
                        .await;
                    }
                }
            }
        }
    }
}

/// Terminal transition decided by precedence: cancelled beats error beats done.
fn terminal_report(cancelled: bool, err: Option<&anyhow::Error>) -> (&'static str, Option<String>) {
    if cancelled {
        ("cancelled", None)
    } else {
        match err {
            Some(e) => ("error", Some(format!("{e:#}"))),
            None => ("done", None),
        }
    }
}

/// Lock helper that recovers from poisoning: a panicked event callback must
/// not wedge every later task's uploads.
fn lock_batcher(b: &Mutex<Batcher>) -> MutexGuard<'_, Batcher> {
    b.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Create the local session mirroring the server-side dispatch row.
/// `task_type="node"` hides it from normal listings (same convention as the
/// web layer); reusing the server's ids verbatim makes cross-machine logs and
/// tasks reconcile 1:1 (two stores, one logical timeline).
///
/// Session REUSE dispatch (P3 dialog resume) re-claims a session this node may
/// already know: an existing local row is kept as-is (its summary/summary_seq
/// drive the resume replay) and only a genuinely fresh id gets a new row.
async fn create_local_meta(
    store: &Arc<dyn Store>,
    workdir: &std::path::Path,
    task: &ClaimedTask,
) -> Result<()> {
    if store.get_session(&task.session_id).await?.is_some() {
        return Ok(());
    }
    let now = now_ms();
    store
        .create_session(&SessionMeta {
            id: task.session_id.clone(),
            title: task.title.clone(),
            agent: task.agent.clone().or_else(|| Some("act".into())),
            model: task.model.clone(),
            autopilot_mode: None,
            workdir_hash: Some(opencoder_core::workdir_hash(workdir)),
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: Some("node".into()),
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire mapping must reuse the session crate's canonical SSE kinds
    /// and payload shapes — byte-identical to what the web layer persists.
    #[test]
    fn to_wire_matches_canonical_sse_shapes() {
        let delta = to_wire(&SessionEvent::TextDelta("hi".into()));
        assert_eq!(delta.sse_kind, "text_delta");
        assert_eq!(delta.payload["text"], "hi");

        let err = to_wire(&SessionEvent::Error("boom".into()));
        assert_eq!(err.sse_kind, "error");
        assert_eq!(err.payload["error"], "boom");

        let done = to_wire(&SessionEvent::Done);
        assert_eq!(done.sse_kind, "done");
        assert_eq!(done.payload, serde_json::json!({}));
    }

    #[test]
    fn terminal_report_precedence() {
        let err = anyhow::anyhow!("boom");
        assert_eq!(terminal_report(false, None), ("done", None));
        assert_eq!(
            terminal_report(false, Some(&err)),
            ("error", Some("boom".into()))
        );
        // Cancelled wins even when the run also surfaced an error.
        assert_eq!(terminal_report(true, Some(&err)), ("cancelled", None));
    }

    #[tokio::test]
    async fn await_flag_fires_on_true_and_ignores_false() {
        let (tx, mut rx) = watch::channel(false);
        tx.send_replace(false);
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            _ = await_flag(&mut rx) => panic!("false must not fire"),
        }
        tx.send_replace(true);
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                panic!("true must fire immediately")
            }
            _ = await_flag(&mut rx) => {}
        }
    }

    #[tokio::test]
    async fn await_flag_parks_when_sender_dies() {
        let (tx, mut rx) = watch::channel(false);
        drop(tx);
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            _ = await_flag(&mut rx) => panic!("dropped sender must not synthesize a cancel"),
        }
    }
}
