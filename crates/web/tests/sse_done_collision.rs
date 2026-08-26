//! Regression test for **P0-1**: the SSE dedup fix in `get_events`.
//!
//! Every `done` event carries the same `{}` payload and, on the live wire,
//! `seq: None`. The OLD dedup seeded its `seen` set from EVERY replayed
//! event's `(kind, data)` fingerprint, so a historical `done` (fingerprint
//! `("done", "{}")`) would silently swallow a later LIVE `done` broadcast —
//! the UI never reset its busy flag, freezing "send".
//!
//! The NEW code seeds `seen` ONLY from events persisted AFTER the subscription
//! baseline (`seq > last_event_seq`). Historical `done` events at or below the
//! baseline no longer collide with the live `done`, so the live one is
//! delivered. This test pins that behaviour: two persisted `done`s (seq 1 & 2)
//! plus one live `done` must yield three `event: done` frames.
//!
//! Under the buggy code the live `done` would be deduped away, leaving only
//! the two replayed frames (count == 2), and this test fails.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use futures::StreamExt;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta};
use opencoder_web::handle::SseEvt;
use serde_json::json;

/// Fresh in-memory AppState (handler is driven directly, no router).
async fn state() -> Arc<opencoder_web::AppState> {
    Arc::new(opencoder_web::AppState {
        client_override: None,
        store: Arc::new(LibsqlStore::open_memory().await.unwrap()),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
    })
}

/// Seed a session row (agent "act", model "m").
async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn live_done_not_suppressed_by_historical_done() {
    let state = state().await;
    let sid = "p01";
    seed(&state, sid).await;

    // (a) Persist TWO historical `done` events, both with the canonical `{}`
    // payload. The store assigns seq via AUTOINCREMENT, so these load back as
    // seq: Some(1) and seq: Some(2); `last_event_seq` (the baseline) is 2.
    let done_payload = json!({});
    for ts in 1..=2 {
        state
            .store
            .append_event(&SessionEventRecord {
                session_id: sid.into(),
                kind: EventKind::Done,
                payload: done_payload.clone(),
                ts,
                seq: None,
                sse_kind: Some("done".into()),
            })
            .await
            .unwrap();
    }

    // (b) Subscribe+replay via the handler. Internally it subscribes the live
    // receiver BEFORE the replay query, then seeds `seen` from persisted rows
    // with seq > baseline (== 2) — i.e. none — so `("done","{}")` is NOT in
    // the set. The OLD code would have pre-filled `seen` with that fingerprint.
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(0) }),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();

    // Grab the handle's tx (created by get_events) so we can broadcast.
    let tx = {
        let map = state.handles.lock().await;
        map.get(sid).unwrap().tx.clone()
    };

    // (c) Broadcast a LIVE `done`: `{}` payload, seq: None — exactly the shape
    // every real `done` takes on the wire. This is the event the OLD dedup
    // dropped due to the `{}` content collision.
    let _ = tx.send(SseEvt {
        kind: "done".into(),
        data: json!({}),
        ts: 3,
        seq: None,
    });

    // (d) Consume the SSE bytes for a bounded window. The broadcast stream
    // stays open forever, so we rely on timeouts + a deadline.
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(400);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }

    // Two replayed `done` frames + the one live `done` => 3. Under the buggy
    // dedup the live `done` was removed (collided with the historical `{}`),
    // leaving only the 2 replayed frames.
    let done_count = text.matches("event: done").count();
    assert!(
        done_count >= 3,
        "live `done` was suppressed by a historical empty-object fingerprint \
         (P0-1 regression); expected >= 3 `event: done` frames (2 replay + 1 \
         live), got {done_count}:\n{text}"
    );
}
