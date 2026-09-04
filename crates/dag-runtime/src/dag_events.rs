//! Run-scoped DAG event pipeline: an unbounded queue of [`DagEventIn`] fed
//! by the (synchronous) scheduling loop, drained by a dedicated uploader
//! task that batches (count cap or time window — same shape as the node
//! crate's event batcher) and pushes batches upstream with bounded retry.
//!
//! Ordering is preserved end-to-end: one channel, one uploader, batches
//! never reordered. A persistently failing uplink degrades to warn-and-drop
//! after three linear-backoff attempts — the server's lost-node sweep
//! converges the run regardless.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::message::now_ms;
use opencoder_dag::{DagEventBatch, DagEventIn};
use opencoder_node::uplink::Uplink;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::warn;

/// Upload as soon as this many events are buffered.
pub const MAX_EVENTS: usize = 8;
/// ... or once this long passed between flush opportunities.
pub const WINDOW: Duration = Duration::from_millis(300);
/// Upload attempts per batch (linear backoff), then warn-and-drop.
const ATTEMPTS: usize = 3;

/// Producer handle for one run's event stream. Cheap to keep on the stack
/// of the scheduling loop; dropping it closes the channel so the uploader
/// flushes the tail in the background. Use [`RunEventSink::close`] when the
/// caller must WAIT for that flush (e.g. before reporting a terminal
/// status).
pub struct RunEventSink {
    run_id: String,
    tx: Option<mpsc::UnboundedSender<DagEventIn>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl RunEventSink {
    /// Spawn the uploader against `uplink`; events enqueue from now on.
    pub fn new(uplink: Arc<Uplink>, run_id: String) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let join = tokio::spawn(uploader(uplink, run_id.clone(), rx));
        Self {
            run_id,
            tx: Some(tx),
            join: Some(join),
        }
    }

    /// Enqueue one event (never blocks; a dead uploader just drops it).
    pub fn emit(&self, ev: DagEventIn) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(ev);
        }
    }

    /// Close the queue and WAIT until the uploader flushed the tail — the
    /// durable ordering point before a terminal status report.
    pub async fn close(mut self) {
        self.tx.take();
        if let Some(join) = self.join.take() {
            if let Err(e) = join.await {
                warn!(run_id = %self.run_id, error = %e, "dag event uploader panicked");
            }
        }
    }
}

impl Drop for RunEventSink {
    fn drop(&mut self) {
        // Dropping the last sender closes the channel; the uploader flushes
        // whatever is buffered with its usual bounded retry, then exits.
        self.tx.take();
    }
}

/// Batching uploader loop: count cap, window tick, or channel close.
async fn uploader(
    uplink: Arc<Uplink>,
    run_id: String,
    mut rx: mpsc::UnboundedReceiver<DagEventIn>,
) {
    let mut buf: Vec<DagEventIn> = Vec::new();
    let mut tick = tokio::time::interval(WINDOW);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(ev) => {
                    buf.push(ev);
                    if buf.len() >= MAX_EVENTS {
                        flush(&uplink, &run_id, &mut buf).await;
                    }
                }
                None => break,
            },
            _ = tick.tick() => {
                if !buf.is_empty() {
                    flush(&uplink, &run_id, &mut buf).await;
                }
            }
        }
    }
    flush(&uplink, &run_id, &mut buf).await;
}

/// One batch upload with bounded retry; the buffer is only cleared on the
/// attempt (retries replay the identical batch, preserving order).
async fn flush(uplink: &Uplink, run_id: &str, buf: &mut Vec<DagEventIn>) {
    if buf.is_empty() {
        return;
    }
    let batch = DagEventBatch {
        run_id: run_id.to_string(),
        events: std::mem::take(buf),
    };
    for attempt in 0..ATTEMPTS {
        match uplink.dag_events(&batch).await {
            Ok(()) => return,
            Err(e) => {
                if attempt + 1 == ATTEMPTS {
                    warn!(
                        run_id,
                        events = batch.events.len(),
                        error = %e,
                        "dag event upload failed after retries; dropping batch"
                    );
                } else {
                    warn!(run_id, attempt, error = %e, "dag event upload retrying");
                    tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
                }
            }
        }
    }
}

/// `run_started` frame. No `node_id`: the runtime does not know the id the
/// runner registered under (the server recorded it at claim time anyway).
pub fn run_started_event(spec_name: &str) -> DagEventIn {
    DagEventIn {
        kind: "run_started".into(),
        step: None,
        payload: json!({ "spec_name": spec_name, "at_ms": now_ms() }),
        at_ms: now_ms(),
    }
}

/// `step_started` frame for one step.
pub fn step_started_event(step: &str) -> DagEventIn {
    DagEventIn {
        kind: "step_started".into(),
        step: Some(step.to_string()),
        payload: json!({ "at_ms": now_ms() }),
        at_ms: now_ms(),
    }
}

/// `step_done` frame; `output` is the ALREADY-truncated snapshot
/// ([`opencoder_dag::artifacts::output_snapshot`]) — full text stays in the
/// node-local artifacts.
pub fn step_done_event(step: &str, ok: bool, error: Option<&str>, output: String) -> DagEventIn {
    let mut payload = json!({ "ok": ok, "output": output });
    if let Some(err) = error {
        payload["error"] = json!(err);
    }
    DagEventIn {
        kind: "step_done".into(),
        step: Some(step.to_string()),
        payload,
        at_ms: now_ms(),
    }
}

/// `run_finished` frame (terminal status + optional error text).
pub fn run_finished_event(status: &str, error: Option<&str>) -> DagEventIn {
    let mut payload = json!({ "status": status });
    if let Some(err) = error {
        payload["error"] = json!(err);
    }
    DagEventIn {
        kind: "run_finished".into(),
        step: None,
        payload,
        at_ms: now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pure constructors must produce exactly the wire vocabulary the
    /// server validates (`DAG_EVENT_KINDS`) and the documented payloads.
    #[test]
    fn event_constructors_match_vocabulary() {
        assert_eq!(run_started_event("etl").kind, "run_started");
        assert_eq!(run_started_event("etl").payload["spec_name"], "etl");
        assert!(run_started_event("etl").payload["at_ms"].as_i64().is_some());

        let s = step_started_event("fetch");
        assert_eq!(
            (s.kind.as_str(), s.step.as_deref()),
            ("step_started", Some("fetch"))
        );

        let d = step_done_event("fetch", false, Some("boom"), "out".into());
        assert_eq!(d.kind, "step_done");
        assert_eq!(d.payload["ok"], false);
        assert_eq!(d.payload["error"], "boom");
        assert_eq!(d.payload["output"], "out");
        // ok steps carry no error key at all.
        let ok = step_done_event("fetch", true, None, String::new());
        assert!(ok.payload.get("error").is_none());

        let f = run_finished_event("cancelled", None);
        assert_eq!(f.kind, "run_finished");
        assert_eq!(f.payload["status"], "cancelled");
        assert!(f.payload.get("error").is_none());
        let fe = run_finished_event("error", Some("bad"));
        assert_eq!(fe.payload["error"], "bad");
    }

    /// Sink close() must flush a tail that never hit the count cap: emit one
    /// event, close, and observe the uploader's batch arrive at a stub
    /// uplink endpoint. Drives the real batching loop with `tokio::pause`
    /// determinism not needed here — close() is the flush trigger.
    #[tokio::test]
    async fn close_flushes_the_tail_in_order() {
        use std::sync::Mutex;
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        // Minimal in-process fake: a local axum app is overkill; instead
        // point the uplink at a real TCP listener that records batches.
        let seen_up = Arc::clone(&seen);
        let app = axum::Router::new().route(
            "/api/nodes/dag/runs/:rid/events",
            axum::routing::post(
                move |axum::extract::Path(rid): axum::extract::Path<String>,
                      axum::Json(batch): axum::Json<DagEventBatch>| async move {
                    assert_eq!(batch.run_id, rid);
                    let mut g = seen_up.lock().unwrap();
                    for ev in &batch.events {
                        g.push(ev.kind.clone());
                    }
                    axum::Json(serde_json::json!({ "accepted": batch.events.len() }))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let uplink = Arc::new(Uplink::new(&format!("http://{addr}"), "t").unwrap());
        let sink = RunEventSink::new(Arc::clone(&uplink), "r1".into());
        sink.emit(run_started_event("etl"));
        sink.emit(step_started_event("a"));
        sink.close().await;

        let got = seen.lock().unwrap().clone();
        assert_eq!(got, vec!["run_started", "step_started"]);
    }
}
