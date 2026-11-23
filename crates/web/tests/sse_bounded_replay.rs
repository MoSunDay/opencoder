//! P0 regression: SSE replay must be a BOUNDED newest window, not the entire
//! persisted history.
//!
//! The original `events_after(after=0)` call in `get_events` had no LIMIT, so
//! every first-connect (and every reconnect-storm tab) cloned the full event
//! history into per-connection memory — OOM spikes on long sessions. The fix
//! replays via the store's bounded `events_tail` (see `sse_replay`:
//! ≤1000 rows / 2 MiB payload budget, newest window kept, ascending order).
//!
//! This test pins the observable contract on the wire: with 1005 persisted
//! events a fresh `after=0` connection replays at most 1000 frames, starting
//! exactly at total-999 (newest window kept, truncation observable via the
//! first replayed seq) and ending at the head seq. Live delivery after the
//! replay window is covered by the other SSE suites and out of scope here.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use futures::StreamExt;
use opencoder_store::{EventKind, SessionEventRecord, SessionMeta, TursoStore};

/// Fresh in-memory AppState (handler is driven directly, no router).
async fn state() -> Arc<opencoder_web::AppState> {
    Arc::new(opencoder_web::AppState {
        config_home: None,
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(Arc::new(
            TursoStore::open_memory().await.unwrap(),
        )),
        store: Arc::new(TursoStore::open_memory().await.unwrap()),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

/// First-connect replay is capped at the newest 1000-event window.
#[tokio::test]
async fn first_connect_replays_bounded_newest_window() {
    let state = state().await;
    let sid = "sse-bounded-replay";
    state
        .store
        .create_session(&SessionMeta {
            id: sid.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
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
            kind: None,
        })
        .await
        .unwrap();

    // 1005 tiny events in one atomic batch (the high-frequency write path).
    let total = 1005usize;
    let batch: Vec<SessionEventRecord> = (0..total)
        .map(|n| SessionEventRecord {
            session_id: sid.into(),
            kind: EventKind::Step,
            payload: serde_json::json!({"n": n}),
            ts: n as i64,
            seq: None,
            sse_kind: Some("step".into()),
        })
        .collect();
    let seqs = state.store.append_events(&batch).await.unwrap();
    assert_eq!(seqs.len(), total);
    let head = seqs[total - 1];

    // First connect with after=0: previously this cloned ALL 1005 events.
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(0) }),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();

    // Drain the replayed frames: stop once the 1000-frame window is in (the
    // live tail then blocks until the deadline). Parse ids from the full text
    // afterwards so chunk-boundary splits cannot corrupt the count.
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let ids = text
            .lines()
            .filter(|l| l.starts_with("id: "))
            .count();
        if ids >= 1000 {
            break;
        }
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(100), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }

    let replayed: Vec<i64> = text
        .lines()
        .filter_map(|l| l.strip_prefix("id: "))
        .filter_map(|v| v.trim().parse::<i64>().ok())
        .collect();
    assert!(
        replayed.len() <= 1000,
        "replay must be bounded by 1000 rows, got {}",
        replayed.len()
    );
    assert_eq!(replayed.len(), 1000, "newest window is exactly 1000 frames");
    // Truncation is observable: first replayed seq = head - 999 (newest kept).
    assert_eq!(replayed.first(), Some(&(head - 999)));
    // The head event itself is present and is the last replayed frame.
    assert_eq!(replayed.last(), Some(&head));
    // Frames stream in ascending seq order (replay + dedup invariants).
    let mut sorted = replayed.clone();
    sorted.sort_unstable();
    assert_eq!(replayed, sorted);
}
