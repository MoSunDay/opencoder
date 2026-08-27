//! P3 message-relay HTTP surface (`/api/nodes/:id/(messages|control_result|dialogs)`).
//!
//! The browser asks the node for a resume-shaped dialog slice; the node
//! uploads it back. This file is the relay: it NEVER persists payload data —
//! the only durable read is `list_node_tasks` for the dialogs index. Split
//! from `api_nodes.rs` for the file-size budget; handlers stay pure
//! composition over the [`ControlHub`] + [`Store`] node API.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_core::node_protocol::{ControlTask, FetchMessagesResult, TASK_KIND_FETCH_MESSAGES};
use serde::Deserialize;
use serde_json::json;

use crate::api::{error_400, error_404, error_500, error_502};
use crate::control_state::{DEFAULT_RELAY_TIMEOUT_MS, MAX_RELAY_TIMEOUT_MS};
use crate::AppState;

/// Browser request body of `POST /api/nodes/:id/messages`. The frozen shape is
/// `{"session_id": "..."}`; `timeout_ms` is an optional test/operator hint,
/// capped by [`MAX_RELAY_TIMEOUT_MS`] (the production default is
/// [`DEFAULT_RELAY_TIMEOUT_MS`]).
#[derive(Deserialize)]
pub struct FetchMessagesBody {
    pub session_id: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// POST /api/nodes/:id/messages — relay one resume-shaped dialog slice.
///
/// Flow: 404 on unknown node -> register waiter -> queue a `fetch_messages`
/// control task -> await the worker's `control_result` upload -> echo the
/// slice (minus control bookkeeping) to the browser. `504` when the worker
/// does not answer inside the window (waiter removed), `502` when it answers
/// `ok:false`. Nothing is written to the store.
pub async fn fetch_node_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<FetchMessagesBody>,
) -> Response {
    if body.session_id.trim().is_empty() {
        return error_400("session_id must not be empty".into());
    }
    match state.store.get_node(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("node not found"),
        Err(e) => return error_500(format!("get_node: {e:#}")),
    }
    let timeout_ms = body
        .timeout_ms
        .unwrap_or(DEFAULT_RELAY_TIMEOUT_MS)
        .clamp(1, MAX_RELAY_TIMEOUT_MS);
    let control_id = opencoder_session::runner::new_id();
    let rx = state.controls.register(&control_id).await;
    state
        .controls
        .push(
            &id,
            ControlTask {
                control_id: control_id.clone(),
                kind: TASK_KIND_FETCH_MESSAGES.into(),
                session_id: body.session_id.clone(),
            },
        )
        .await;

    let wait = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await;
    match wait {
        // Worker answered: ok -> echo the slice; failure -> 502 with reason.
        Ok(Ok(result)) => {
            if !result.ok {
                return error_502(format!(
                    "node failed to fetch messages: {}",
                    result.error.unwrap_or_else(|| "unknown".into())
                ));
            }
            Json(json!({
                "session_id": result.session_id,
                "summary": result.summary,
                "summary_seq": result.summary_seq,
                "messages": result.messages,
            }))
            .into_response()
        }
        // Waiter dropped without a resolve (hub purge / receiver gone).
        Ok(Err(_)) => {
            state.controls.abandon(&control_id).await;
            error_502("control result channel closed".into())
        }
        // No answer inside the window: remove the waiter so a late upload
        // resolves to `false`, and tell the browser the node timed out.
        Err(_elapsed) => {
            state.controls.abandon(&control_id).await;
            (
                axum::http::StatusCode::GATEWAY_TIMEOUT,
                Json(json!({
                    "error": "node did not deliver the dialog slice in time",
                    "timeout_ms": timeout_ms,
                })),
            )
                .into_response()
        }
    }
}

/// POST /api/nodes/:id/control_result — worker uploads a control result.
///
/// Always 200: an unknown/stale `control_id` (timed-out browser, node delete,
/// duplicated upload) must not look like an error to the worker, which would
/// only retry a delivery nobody waits for. `resolved` says whether anyone
/// woke up.
pub async fn post_control_result(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(result): Json<FetchMessagesResult>,
) -> Response {
    let _ = &id; // route symmetry: the worker posts under its own node id
    let control_id = result.control_id.clone();
    let resolved = state.controls.resolve(&control_id, result).await;
    Json(json!({ "resolved": resolved })).into_response()
}

/// One grouped dialog in `GET /api/nodes/:id/dialogs`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DialogSummary {
    pub session_id: String,
    /// Most recent task title (the console's dialog headline); null when the
    /// node's tasks carry none.
    pub title: Option<String>,
    pub first_created_at: i64,
    pub last_created_at: i64,
    pub task_count: usize,
}

/// Pure grouping of a node's task rows into dialogs: one entry per distinct
/// `session_id`, title = the newest non-null title, bounds = min/max
/// `created_at`, ordered by `last_created_at` DESC (newest dialog first).
pub fn group_dialogs(tasks: &[opencoder_store::NodeTaskRecord]) -> Vec<DialogSummary> {
    let mut grouped: BTreeMap<String, (Option<String>, i64, i64, usize)> = BTreeMap::new();
    for t in tasks {
        let entry = grouped
            .entry(t.session_id.clone())
            .or_insert_with(|| (None, t.created_at, t.created_at, 0));
        // Tasks arrive newest-first; only the FIRST seen (newest) non-null
        // title wins.
        if entry.0.is_none() {
            entry.0 = t.title.clone();
        }
        entry.1 = entry.1.min(t.created_at);
        entry.2 = entry.2.max(t.created_at);
        entry.3 += 1;
    }
    let mut out: Vec<DialogSummary> = grouped
        .into_iter()
        .map(|(session_id, (title, first, last, count))| DialogSummary {
            session_id,
            title,
            first_created_at: first,
            last_created_at: last,
            task_count: count,
        })
        .collect();
    out.sort_by(|a, b| {
        b.last_created_at
            .cmp(&a.last_created_at)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    out
}

/// GET /api/nodes/:id/dialogs — the node's dialogs index for the console.
pub async fn list_dialogs(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.store.get_node(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("node not found"),
        Err(e) => return error_500(format!("get_node: {e:#}")),
    }
    let tasks = match state.store.list_node_tasks(&id, 200).await {
        Ok(t) => t,
        Err(e) => return error_500(format!("list_node_tasks: {e:#}")),
    };
    Json(json!({ "dialogs": group_dialogs(&tasks) })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::NodeTaskRecord;

    fn task(id: &str, session_id: &str, title: Option<&str>, created_at: i64) -> NodeTaskRecord {
        NodeTaskRecord {
            id: id.into(),
            node_id: "n".into(),
            session_id: session_id.into(),
            title: title.map(str::to_string),
            prompt: "p".into(),
            agent: None,
            model: None,
            status: opencoder_store::NodeTaskStatus::Done,
            error: None,
            cancel_requested: false,
            created_at,
            claimed_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn group_dialogs_counts_bounds_and_orders_by_recency() {
        let tasks = vec![
            task("t3", "s-a", Some("newest"), 300),
            task("t1", "s-b", Some("older b"), 100),
            task("t2", "s-a", Some("older a"), 150),
            task("t4", "s-a", None, 50),
        ];
        let dialogs = group_dialogs(&tasks);
        assert_eq!(dialogs.len(), 2, "two distinct sessions");
        // Newest dialog first.
        assert_eq!(dialogs[0].session_id, "s-a");
        assert_eq!(dialogs[1].session_id, "s-b");
        let a = &dialogs[0];
        assert_eq!(a.task_count, 3);
        assert_eq!(a.first_created_at, 50, "min created_at wins");
        assert_eq!(a.last_created_at, 300);
        assert_eq!(a.title.as_deref(), Some("newest"), "newest task title");
        let b = &dialogs[1];
        assert_eq!(b.task_count, 1);
        assert_eq!(b.first_created_at, 100);
        assert_eq!(b.last_created_at, 100);
        assert_eq!(b.title.as_deref(), Some("older b"));
    }

    #[test]
    fn group_dialogs_tie_breaks_by_session_id_and_handles_empty() {
        let mut tasks = vec![task("t1", "s-z", None, 200), task("t2", "s-a", None, 200)];
        // Same recency: stable alphabetical tiebreak.
        let dialogs = group_dialogs(&tasks);
        assert_eq!(dialogs[0].session_id, "s-a");
        assert_eq!(dialogs[1].session_id, "s-z");
        assert_eq!(dialogs[0].title, None);

        tasks.clear();
        assert!(group_dialogs(&tasks).is_empty(), "no rows -> no dialogs");
    }

    #[test]
    fn group_dialogs_picks_first_non_null_title_scanning_newest_first() {
        // Reverse-chronological input where the newest rows carry no title.
        let tasks = vec![
            task("t2", "s", None, 200),
            task("t1", "s", Some("kept"), 100),
        ];
        let dialogs = group_dialogs(&tasks);
        assert_eq!(dialogs[0].title.as_deref(), Some("kept"));
    }
}
