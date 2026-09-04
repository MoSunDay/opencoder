//! Browser SSE stream for one DAG run: `GET /api/dag/runs/:rid/events`.
//!
//! Mirrors `sse_nodes.rs` (replay-from-store, live broadcast, Last-Event-ID,
//! keep-alive and finalize-on-drop) but sources frames from the [`DagHub`]
//! and replays `dag_events` rows. Every frame — replayed or live — is a
//! [`DagEventView`] carrying its row `seq`, so overlap dedup is exact-seq
//! (no content fingerprint tier is needed).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream::StreamExt;
use opencoder_dag::DagEventView;
use serde::Deserialize;

use crate::api::{error_404, error_500};
use crate::dag_state::shared_dag_hub;
use crate::AppState;

/// Replay slice bound for the persisted prefix (same budget shape as the
/// node-task stream; a browser reconnects with `Last-Event-ID` for more).
const REPLAY_LIMIT: u32 = 1000;

#[derive(Deserialize, Default)]
pub struct DagEventsQuery {
    pub after: Option<i64>,
}

/// Store row → wire frame. `DagEventRecord.seq` is always Some for rows read
/// back from the table; the default only satisfies the record type.
fn record_to_view(r: opencoder_store::DagEventRecord) -> DagEventView {
    DagEventView {
        seq: r.seq.unwrap_or(0),
        kind: r.kind,
        step: r.step,
        payload: r.payload,
        at_ms: r.at_ms,
    }
}

fn view_to_frame(v: DagEventView) -> Event {
    let data = serde_json::to_string(&v).unwrap_or_else(|_| "{}".into());
    // event name = kind, id = seq (the Last-Event-ID reconnect cursor).
    Event::default()
        .event(v.kind)
        .data(data)
        .id(v.seq.to_string())
}

/// Stream adapter that runs a finalizer once when dropped (client disconnect
/// or natural end) — same shape as `sse_nodes::FinalizeOnDrop`, releasing the
/// hub receiver slot.
struct FinalizeOnDrop<S> {
    inner: std::pin::Pin<Box<S>>,
    on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl<S: futures::Stream> FinalizeOnDrop<S> {
    fn new(stream: S, on_drop: impl FnOnce() + Send + Sync + 'static) -> Self {
        FinalizeOnDrop {
            inner: Box::pin(stream),
            on_drop: Some(Box::new(on_drop)),
        }
    }
}

impl<S: futures::Stream> futures::Stream for FinalizeOnDrop<S> {
    type Item = S::Item;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl<S> Drop for FinalizeOnDrop<S> {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

/// SSE stream: replay persisted run events after the cursor (`?after=` or
/// the SSE-standard `Last-Event-ID` header, unparseable → 0), then forward
/// live hub frames. Unknown runs fail fast with 404 (mirroring
/// `sse_nodes`), never a hanging empty stream.
pub async fn get_dag_run_events(
    State(state): State<Arc<AppState>>,
    Path(rid): Path<String>,
    Query(q): Query<DagEventsQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let header_after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let after = q.after.or(header_after).unwrap_or(0);

    match state.store.get_dag_run(&rid).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("dag run not found"),
        Err(e) => return error_500(format!("get_dag_run: {e:#}")),
    }

    // Subscribe FIRST, then snapshot the replay window — closes the
    // subscribe/query race; live frames at or below the replay head are
    // duplicates of the overlap window and get dropped (exact-seq dedup).
    let (rx, _created) = shared_dag_hub().subscribe(&rid).await;
    let persisted: Vec<DagEventView> = state
        .store
        .dag_events_after(&rid, after, REPLAY_LIMIT)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(record_to_view)
        .collect();
    let max_replay_seq: i64 = persisted.iter().map(|v| v.seq).max().unwrap_or(after);

    let replay = futures::stream::iter(persisted);
    let live = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|r| async move { r.ok() })
        .filter_map(move |v| {
            let keep = v.seq > max_replay_seq;
            async move { keep.then_some(v) }
        });
    let merged = replay
        .chain(live)
        .map(|v| Ok::<_, std::convert::Infallible>(view_to_frame(v)));

    let hub = shared_dag_hub();
    let rid_final = rid.clone();
    let guarded = FinalizeOnDrop::new(merged, move || {
        tokio::spawn(async move { hub.cleanup(&rid_final).await });
    });

    Sse::new(guarded)
        .keep_alive(KeepAlive::default())
        .into_response()
}
