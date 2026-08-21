//! SSE events endpoints, split out of `api.rs` to respect the file-size
//! budget: `GET /events` (persisted replay + live broadcast with two-tier
//! dedup, subscriber-slot drop guard) and `GET /seq` (reconnect cursor).
//! Re-exported through `crate::api` so handler paths stay stable.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::handle::{SessionHandle, SseEvt};
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct EventsQuery {
    pub after: Option<i64>,
}

/// SSE stream: replay persisted events `after` the cursor, then forward the
/// live broadcast. Slow clients skip lagged events (backpressure never blocks
/// the runner); a missing live handle still yields the replay window.
///
/// The cursor comes from `?after=` or, when absent, the SSE-standard
/// `Last-Event-ID` header (unparseable values ignored → fall back to 0).
pub async fn get_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    // `Last-Event-ID` has no constant in the `http` crate; parse by name.
    let header_after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let after = q.after.or(header_after).unwrap_or(0);

    // Reject non-existent sessions: otherwise a get-or-created handle would
    // subscribe to a broadcast that never fires, hanging the SSE stream forever.
    match state.store.get_session(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("session not found"),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    }

    // Subscribe FIRST, then query persisted events. This closes the race where
    // an event broadcast between query and subscribe is lost (not yet
    // persisted at query time, not received via broadcast). With subscribe-first
    // every post-subscribe broadcast is captured by the live stream; any overlap
    // with the replay window is deduplicated below.
    let (rx, created) = {
        let mut map = state.handles.lock().await;
        let created = !map.contains_key(&id);
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        // Track this subscriber so the handle this request may have created is
        // evicted (see `release_events_subscriber`) once everyone disconnects.
        handle.subscribers.fetch_add(1, Ordering::SeqCst);
        (handle.tx.subscribe(), created)
    };

    // P0-1: Capture the persisted-seq baseline BEFORE querying `events_after`.
    // `last_event_seq` returns the current max persisted seq; reading it AFTER
    // `events_after` (as the original code did) guarantees `baseline >=
    // max(seq)`, so the `seq > baseline` filter below is ALWAYS false and `seen`
    // (the tier-(2) content-dedup set) stays permanently empty. Snapshotting it
    // here — immediately after subscribing — means any event persisted in the
    // window between this snapshot and the `events_after` query (seq > baseline)
    // is a genuine subscribe/query overlap-window event that must seed `seen`.
    let baseline = state.store.last_event_seq(&id).await.unwrap_or(-1);

    let persisted: Vec<SseEvt> = state
        .store
        .events_after(&id, after)
        .await
        .map(|records| {
            records
                .into_iter()
                .map(|r| SseEvt {
                    kind: r
                        .sse_kind
                        .clone()
                        .unwrap_or_else(|| crate::api::event_kind_str(r.kind).to_string()),
                    data: r.payload,
                    ts: r.ts,
                    seq: r.seq,
                })
                .collect()
        })
        .unwrap_or_default();

    // Dedup live broadcast events against the replayed (persisted) window:
    // two-tier decision (exact seq, then content fingerprint) + overlap-window
    // seeding and the first-forwarded-`done` TTL live in `sse_dedup`.
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

    // Wrap in a drop guard so that when the client disconnects (or the stream
    // ends) this request's subscriber slot is released and, if it created the
    // handle and nothing remains, the handle is evicted — preventing unbounded
    // handle-map growth on a long-running server.
    let guarded = crate::handle::DropGuardStream::new(merged, move || {
        crate::handle::release_events_subscriber(state.handles.clone(), id, created)
    });

    Sse::new(guarded)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Highest persisted event seq for a session (0 if none). A remote client uses
/// this to snapshot before posting a prompt so it only streams the events
/// produced by its own turn.
pub async fn get_event_seq(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let seq = state.store.last_event_seq(&id).await.unwrap_or(0);
    Json(json!({ "id": id, "seq": seq }))
}

fn error_404(msg: &str) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
