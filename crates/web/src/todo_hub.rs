//! `GET /api/todo/workflows/:id/events` — SSE tail of a workflow's persisted
//! event log. The producer is whatever process drives the Runtime (this
//! server's spawned runs AND the CLI in another process); there is no
//! in-process broadcast channel, so the stream is a store-polling loop:
//! replay `todo_events_after(id, cursor)` immediately, then every 500ms,
//! forwarding each new event as an SSE frame named by its kind with the seq
//! as `id:` (the `Last-Event-ID` reconnect cursor — same convention as
//! `/api/sessions/:id/events`). Terminal frames (`workflow_completed` /
//! `workflow_failed`) close the stream; store errors are logged and retried.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::VecDeque;
use std::sync::Arc;

use opencoder_store::TodoEventRecord;

use crate::api_todo_util::{error_404, error_500};
use crate::AppState;

/// Poll cadence for the event tail.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Deserialize, Default)]
pub struct TodoEventsQuery {
    pub after: Option<i64>,
}

/// Poller state: pending events already fetched (a poll can return a burst),
/// the resume cursor, and the close-after-terminal flag. Data-only (no impl
/// blocks) — the loop lives in the `unfold` closure below.
struct PollState {
    state: Arc<AppState>,
    id: String,
    cursor: i64,
    buffer: VecDeque<TodoEventRecord>,
    closing: bool,
}

/// SSE endpoint. Missing workflow ⇒ plain 404 JSON (before the stream opens);
/// otherwise the response stays open until a terminal event or disconnect.
pub async fn workflow_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<TodoEventsQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    match state.store.get_todo_workflow(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404(&format!("workflow 不存在: {id}")),
        Err(e) => return error_500(format!("get workflow: {e:#}")),
    }
    // Same dual-cursor rule as the session events endpoint: `?after=` wins,
    // the SSE-standard `Last-Event-ID` header is the fallback, floor 0.
    let header_after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let cursor = q.after.or(header_after).unwrap_or(0).max(0);

    let seed = PollState {
        state,
        id,
        cursor,
        buffer: VecDeque::new(),
        closing: false,
    };
    let stream = futures::stream::unfold(seed, |mut poll| async move {
        if poll.closing {
            return None;
        }
        if poll.buffer.is_empty() {
            // Poll until at least one new event shows up. Store hiccups
            // (locked table, transient IO) degrade to a warn + retry, never
            // an error frame: the workflow log is append-only, so the cursor
            // makes every retry idempotent.
            loop {
                match poll
                    .state
                    .store
                    .todo_events_after(&poll.id, poll.cursor)
                    .await
                {
                    Ok(events) if !events.is_empty() => {
                        for event in events {
                            if let Some(seq) = event.seq {
                                poll.cursor = poll.cursor.max(seq);
                            }
                            poll.buffer.push_back(event);
                        }
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(
                        workflow_id = %poll.id,
                        error = %format!("{e:#}"),
                        "todo event poll failed; retrying"
                    ),
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
        let event = poll.buffer.pop_front()?;
        if matches!(
            event.kind.as_str(),
            "workflow_completed" | "workflow_failed"
        ) {
            poll.closing = true;
        }
        let data = serde_json::to_string(&event.payload).unwrap_or_else(|_| "{}".into());
        let mut frame = Event::default().event(event.kind.clone()).data(data);
        if let Some(seq) = event.seq {
            frame = frame.id(seq.to_string());
        }
        Some((Ok::<_, Infallible>(frame), poll))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
