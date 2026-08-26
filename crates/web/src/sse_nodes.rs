//! Browser SSE stream for node-task sessions: `GET /api/nodes/tasks/:tid/events`.
//!
//! Mirrors `api_events.rs` (subscribe → baseline → replay → live with two-tier
//! dedup) but sources the live broadcast from the [`NodeHub`] instead of a
//! drain-owned `SessionHandle`, and the finalizer releases a hub receiver
//! instead of a handle-map subscriber slot.
//!
//! [`NodeHub`]: crate::nodes_state::NodeHub

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream::StreamExt;
use opencoder_core::SseEvt;
use opencoder_store::SessionEventRecord;
use serde::Deserialize;

use crate::api::{error_404, error_500};
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct NodeEventsQuery {
    pub after: Option<i64>,
}

/// Persisted row → SSE envelope. Same mapping rule as `api_events`: the
/// granular `sse_kind` wins when present; pre-migration rows fall back to the
/// coarse kind's canonical string.
fn persisted_to_evt(r: SessionEventRecord) -> SseEvt {
    SseEvt {
        kind: r
            .sse_kind
            .clone()
            .unwrap_or_else(|| crate::api::event_kind_str(r.kind).to_string()),
        data: r.payload,
        ts: r.ts,
        seq: r.seq,
    }
}

/// Stream adapter that runs a finalizer once when dropped (client disconnect
/// or natural end). Same idea as `handle::DropGuardStream`, but node streams
/// hold no HandleMap slot — they release their [`NodeHub`] receiver instead.
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

/// SSE stream: replay persisted task-session events after the cursor, then
/// forward live uploads broadcast on the hub. The cursor comes from `?after=`
/// or, when absent, the SSE-standard `Last-Event-ID` header (unparseable → 0).
pub async fn get_node_task_events(
    State(state): State<Arc<AppState>>,
    Path(tid): Path<String>,
    Query(q): Query<NodeEventsQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let header_after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let after = q.after.or(header_after).unwrap_or(0);

    // Fail fast on unknown tasks: subscribing first would leave us waiting on a
    // channel nobody ever writes to, hanging the stream forever.
    let sid = match state.store.get_node_task(&tid).await {
        Ok(Some(t)) => t.session_id,
        Ok(None) => return error_404("task not found"),
        Err(e) => return error_500(format!("get_node_task: {e:#}")),
    };

    // Subscribe FIRST, then snapshot the persisted-seq baseline and replay —
    // this closes the subscribe/query race exactly like the primary /events.
    let (rx, _created) = state.nodes.subscribe(&sid).await;
    let baseline = state.store.last_event_seq(&sid).await.unwrap_or(-1);

    let persisted: Vec<SseEvt> = state
        .store
        .events_after(&sid, after)
        .await
        .map(|records| records.into_iter().map(persisted_to_evt).collect())
        .unwrap_or_default();

    // Two-tier dedup of live events against the replayed window (exact seq,
    // then content fingerprint): shared logic in `sse_dedup`.
    let max_replay_seq: i64 = persisted.iter().filter_map(|e| e.seq).max().unwrap_or(-1);
    let seen = crate::sse_dedup::seed_seen(&persisted, baseline);

    let replay = futures::stream::iter(persisted);
    let live =
        tokio_stream::wrappers::BroadcastStream::new(rx)
            .filter_map(|r| async move { crate::api::map_broadcast_result(r) })
            .filter_map({
                let seen = Arc::clone(&seen);
                move |evt| {
                    let seen = Arc::clone(&seen);
                    async move {
                        crate::sse_dedup::forward_live(&evt, &seen, max_replay_seq).then_some(evt)
                    }
                }
            });
    let merged = replay.chain(live).map(|evt| {
        let data = serde_json::to_string(&evt.data).unwrap_or_else(|_| "{}".into());
        Ok::<_, std::convert::Infallible>(Event::default().event(evt.kind).data(data))
    });

    // Finalizer: on disconnect, release our claim; the hub evicts the channel
    // only when the last receiver is gone. The cleanup lock is tokio-async, so
    // detach it instead of blocking drop.
    let hub = Arc::clone(&state.nodes);
    let sid_final = sid.clone();
    let guarded = FinalizeOnDrop::new(merged, move || {
        tokio::spawn(async move { hub.cleanup(&sid_final).await });
    });

    Sse::new(guarded)
        .keep_alive(KeepAlive::default())
        .into_response()
}
