use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use opencoder_core::fleet::RpcReply;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::VecDeque, convert::Infallible, future::Future, pin::Pin, sync::Arc, time::Duration,
};

/// Poll interval for the first re-fetch once the local queue drains.
const POLL_FLOOR: Duration = Duration::from_millis(300);
/// Poll interval cap after consecutive empty pages. Bounds the control
/// plane to node RPC rate of idle spectator streams while keeping new-event
/// visibility at one second.
const POLL_CAP: Duration = Duration::from_secs(1);

/// Poll interval after `empty_polls` consecutive pages delivered nothing
/// new: ramps from the floor to the cap. Any delivered frame resets the
/// ladder (see the unfold loop).
fn poll_delay(empty_polls: u32) -> Duration {
    if empty_polls == 0 {
        POLL_FLOOR
    } else {
        POLL_CAP
    }
}

#[derive(Deserialize)]
pub struct Cursor {
    pub after: Option<i64>,
}

/// One page fetch: cursor in, node reply out. Owned captures keep the unfold
/// state `'static` so every SSE endpoint shares the same paging loop.
type PageFetch = Arc<dyn Fn(i64) -> Pin<Box<dyn Future<Output = RpcReply> + Send>> + Send + Sync>;

/// Resume from the newest cursor supplied by the caller or EventSource.
fn cursor_after(query: &Cursor, headers: &HeaderMap) -> i64 {
    query.after.unwrap_or(0).max(
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok()?.parse::<i64>().ok())
            .unwrap_or(0),
    )
}

/// Turn a paged node query into an SSE stream: emit queued frames, refetch
/// after [`poll_delay`] when the queue drains (the interval ramps to its cap
/// across consecutive empty pages and resets on any delivered frame), and
/// end once the node reports `finished` with no further page. A non-200
/// poll becomes one `error` frame.
fn sse(
    first: RpcReply,
    fetch: PageFetch,
    after: i64,
    lifecycle: Arc<crate::release::Lifecycle>,
) -> Response {
    if first.status != 200 {
        return super::response(first);
    }
    let stream = futures::stream::unfold(
        (
            lifecycle,
            fetch,
            after,
            VecDeque::<Value>::new(),
            Some(first.body),
            false,
            0u32,
            false,
        ),
        |(lifecycle, fetch, mut cursor, mut queue, mut page, mut ended, mut empty_polls, end_sent)| async move {
            if end_sent {
                return None;
            }
            loop {
                if lifecycle.retiring.load(std::sync::atomic::Ordering::SeqCst) {
                    let event = Event::default()
                        .event("reconnect")
                        .id(cursor.to_string())
                        .retry(Duration::from_millis(100))
                        .data("release switch");
                    return Some((
                        Ok(event),
                        (lifecycle, fetch, cursor, queue, None, false, empty_polls, true),
                    ));
                }
                if let Some(frame) = queue.pop_front() {
                    let frame = normalize_frame(frame);
                    let seq = frame["seq"].as_i64().unwrap_or(cursor);
                    cursor = cursor.max(seq);
                    let event = Event::default()
                        .id(seq.to_string())
                        .event(frame["kind"].as_str().unwrap_or("status"))
                        .json_data(&frame["data"])
                        .expect("JSON event");
                    // Frames flowing means an active stream: reset the
                    // empty-poll ladder so the next re-fetch stays fast.
                    empty_polls = 0;
                    return Some((
                        Ok::<_, Infallible>(event),
                        (lifecycle, fetch, cursor, queue, page, ended, empty_polls, false),
                    ));
                }
                if ended {
                    let event = Event::default()
                        .event("stream_end")
                        .json_data(serde_json::json!({"finished":true}))
                        .expect("JSON stream end");
                    return Some((
                        Ok(event),
                        (lifecycle, fetch, cursor, queue, None, true, empty_polls, true),
                    ));
                }
                let body = match page.take() {
                    Some(body) => body,
                    None => {
                        let reply = tokio::select! {
                            reply = async { tokio::time::sleep(poll_delay(empty_polls)).await; fetch(cursor).await } => reply,
                            _ = lifecycle.retired() => continue,
                        };
                        if reply.status != 200 {
                            let event = Event::default()
                                .event("error")
                                .json_data(reply.body)
                                .expect("JSON error");
                            return Some((
                                Ok(event),
                                (lifecycle, fetch, cursor, queue, None, true, empty_polls, true),
                            ));
                        }
                        reply.body
                    }
                };
                let rows = body["events"].as_array().cloned().unwrap_or_default();
                ended = body["finished"].as_bool().unwrap_or(false)
                    && !body["more"].as_bool().unwrap_or(false);
                let delivered = queue.len();
                queue.extend(
                    rows.into_iter()
                        .filter(|row| row["seq"].as_i64().is_some_and(|seq| seq > cursor)),
                );
                if queue.len() == delivered {
                    empty_polls = empty_polls.saturating_add(1);
                }
            }
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
        .into_response()
}

/// The generic execution stream uses `{seq, kind, data}` rows. V3 scheduler
/// events are intentionally typed index rows, so adapt them at the transport
/// boundary without changing the paged API or copying execution bodies.
fn normalize_frame(mut frame: Value) -> Value {
    if frame.get("kind").is_none() && frame.get("event_type").is_some() {
        let kind = frame["event_type"].clone();
        let data = frame.clone();
        frame["kind"] = kind;
        frame["data"] = data;
    }
    frame
}

pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let after = cursor_after(&query, &headers);
    let first = super::executions::events_id(&state, &id, after).await;
    let lifecycle = state.lifecycle.clone();
    let fetch: PageFetch = Arc::new(move |cursor| {
        let state = state.clone();
        let id = id.clone();
        Box::pin(async move { super::executions::events_id(&state, &id, cursor).await })
    });
    sse(first, fetch, after, lifecycle)
}

/// One DAG step's event stream (`/api/dag/runs/:id/steps/:step/events`).
/// The node resolves the step's event source (child session for agent steps,
/// filtered run session otherwise), so the paging loop is the run-level one.
pub async fn dag_step_events(
    State(state): State<Arc<AppState>>,
    Path((id, step)): Path<(String, String)>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let after = cursor_after(&query, &headers);
    let first = super::executions::dag_step_events_id(&state, &id, &step, after).await;
    let lifecycle = state.lifecycle.clone();
    let fetch: PageFetch = Arc::new(move |cursor| {
        let state = state.clone();
        let id = id.clone();
        let step = step.clone();
        Box::pin(
            async move { super::executions::dag_step_events_id(&state, &id, &step, cursor).await },
        )
    });
    sse(first, fetch, after, lifecycle)
}

/// One selected dynamic instance uses the same bounded, replayable SSE transport.
pub async fn dag_instance_events(
    State(state): State<Arc<AppState>>,
    Path((id, step, index)): Path<(String, String, usize)>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let after = cursor_after(&query, &headers);
    let fetch: PageFetch = {
        let state = state.clone();
        Arc::new(move |cursor| {
            let state = state.clone();
            let id = id.clone();
            let step = step.clone();
            Box::pin(async move {
                super::executions::for_id(&state, &id, |execution| {
                    opencoder_core::fleet::NodeOperation::DagInstanceEvents {
                        execution,
                        step,
                        index,
                        after: cursor,
                    }
                })
                .await
            })
        })
    };
    let first = fetch(after).await;
    sse(first, fetch, after, state.lifecycle.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_delay_ramps_on_empty_pages_and_caps_at_one_second() {
        assert_eq!(poll_delay(0), POLL_FLOOR);
        assert_eq!(poll_delay(1), POLL_CAP, "an empty page steps to the cap");
        assert_eq!(poll_delay(9), POLL_CAP, "the ladder never grows past the cap");
    }
}
