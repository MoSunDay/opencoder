use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::VecDeque, convert::Infallible, sync::Arc, time::Duration};

#[derive(Deserialize)]
pub struct Cursor {
    pub after: Option<i64>,
}
pub async fn events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let after = query
        .after
        .or_else(|| {
            headers
                .get("last-event-id")
                .and_then(|v| v.to_str().ok()?.parse().ok())
        })
        .unwrap_or(0);
    let first = super::executions::events_id(&state, &id, after).await;
    if first.status != 200 {
        return super::response(first);
    }
    let stream = futures::stream::unfold(
        (
            state,
            id,
            after,
            VecDeque::<Value>::new(),
            Some(first.body),
            false,
        ),
        |(state, id, mut cursor, mut queue, mut page, mut ended)| async move {
            loop {
                if let Some(frame) = queue.pop_front() {
                    let seq = frame["seq"].as_i64().unwrap_or(cursor);
                    cursor = cursor.max(seq);
                    let event = Event::default()
                        .id(seq.to_string())
                        .event(frame["kind"].as_str().unwrap_or("status"))
                        .json_data(&frame["data"])
                        .expect("JSON event");
                    return Some((
                        Ok::<_, Infallible>(event),
                        (state, id, cursor, queue, page, ended),
                    ));
                }
                if ended {
                    return None;
                }
                let body = match page.take() {
                    Some(body) => body,
                    None => {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        let reply = super::executions::events_id(&state, &id, cursor).await;
                        if reply.status != 200 {
                            let event = Event::default()
                                .event("error")
                                .json_data(reply.body)
                                .expect("JSON error");
                            return Some((Ok(event), (state, id, cursor, queue, None, true)));
                        }
                        reply.body
                    }
                };
                let rows = body["events"].as_array().cloned().unwrap_or_default();
                ended = body["finished"].as_bool().unwrap_or(false)
                    && !body["more"].as_bool().unwrap_or(false);
                queue.extend(
                    rows.into_iter()
                        .filter(|row| row["seq"].as_i64().is_some_and(|seq| seq > cursor)),
                );
            }
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(5)))
        .into_response()
}
