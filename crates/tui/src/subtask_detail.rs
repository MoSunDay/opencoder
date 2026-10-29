//! `/subtasks` — fullscreen detail view over this session's one-shot
//! remote subtask registry.
//!
//! The list band shows the registry entries owned by the parent session's
//! harness runtime; the detail pane shows the selected subtask's question,
//! server-side process timeline (`r` fetches it), result summary and token
//! cost. Keys: `j/k` select, `PgUp/PgDn` scroll, `r` refresh snapshots, `c`
//! cancel, `Enter` descend into the child session (agent/operator kinds
//! have a local `tui_remote_*` bookmark session), `Ctrl+Enter` append the
//! finished result back to the main task, `Esc` close.
//!
//! Server-side reads run on spawned tasks and land through one-shot
//! channels (the `remote::selection` pattern) so the event loop never
//! blocks on HTTP.

#[path = "subtask_detail_render.rs"]
mod render;
pub(crate) use render::render as render_detail;

use std::cell::Cell;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{harness::SubtaskRef, Config};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::remote::client::ServerClient;

/// Timeline rows kept per subtask snapshot (server frames are unbounded).
const TIMELINE_MAX_ROWS: usize = 200;
/// Event pages fetched per timeline (each page is bounded server-side too).
const TIMELINE_MAX_PAGES: usize = 5;

/// One subtask's server-side snapshot for the detail view.
pub(crate) struct SubtaskSnapshot {
    pub execution_id: String,
    pub status: String,
    pub summary: Option<String>,
    pub tokens: u64,
    /// Bounded process timeline rows (newest last).
    pub timeline: Vec<String>,
}

/// Fullscreen `/subtasks` view state: an owned registry copy plus the live
/// server snapshots fetched on demand.
pub(crate) struct SubtaskView {
    /// Registry entries (owned copy; refreshed by `r` / watcher updates).
    pub entries: Vec<SubtaskRef>,
    pub selected: usize,
    pub scroll: u32,
    /// Live server snapshots fetched by `r` (keyed by execution_id).
    pub snapshots: HashMap<String, SubtaskSnapshot>,
    /// Detail-pane content height of the last rendered frame (set by the
    /// renderer; bounds scrolling). A `Cell` keeps the renderer shared-ref.
    pub last_detail_lines: Cell<u32>,
    refresh: Option<oneshot::Receiver<anyhow::Result<Vec<SubtaskSnapshot>>>>,
    cancel: Option<oneshot::Receiver<anyhow::Result<()>>>,
}

impl SubtaskView {
    pub(crate) fn new(entries: Vec<SubtaskRef>) -> Self {
        let selected = entries.len().saturating_sub(1);
        SubtaskView {
            entries,
            selected,
            scroll: 0,
            snapshots: HashMap::new(),
            last_detail_lines: Cell::new(0),
            refresh: None,
            cancel: None,
        }
    }

    /// The currently selected registry entry, if any.
    pub(crate) fn selected_entry(&self) -> Option<&SubtaskRef> {
        self.entries.get(self.selected)
    }

    /// Move the selection by `delta` (clamped to the list); resets scroll.
    pub(crate) fn move_selection(&mut self, delta: i32) {
        self.selected = if self.entries.is_empty() {
            0
        } else {
            let max = self.entries.len() as i32 - 1;
            ((self.selected as i32) + delta).clamp(0, max) as usize
        };
        self.scroll = 0;
    }

    /// Scroll the detail pane up by `step` (bounded at 0).
    pub(crate) fn scroll_up(&mut self, step: u32) {
        self.scroll = self.scroll.saturating_sub(step);
    }

    /// Scroll the detail pane down by `step`, bounded at the content height
    /// the renderer recorded for the last frame.
    pub(crate) fn scroll_down(&mut self, step: u32) {
        self.scroll = (self.scroll + step).min(self.last_detail_lines.get());
    }

    /// Fold one status/summary/tokens update into the matching registry
    /// entry and snapshot (watcher deliveries and refresh merges). The
    /// summary is bounded on a char boundary like every display copy.
    pub(crate) fn apply_update(
        &mut self,
        id: &str,
        status: &str,
        summary: Option<&str>,
        tokens: u64,
    ) {
        let bounded = summary.map(crate::subtask_registry::bound_summary);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.execution_id == id) {
            entry.status = status.to_string();
            entry.result_summary = bounded.clone();
            entry.tokens = tokens;
        }
        if let Some(snapshot) = self.snapshots.get_mut(id) {
            snapshot.status = status.to_string();
            snapshot.summary = bounded;
            snapshot.tokens = tokens;
        }
    }

    /// Start a background snapshot refresh for every registered id (no-op
    /// while one is in flight or the registry is empty).
    pub(crate) fn start_refresh(&mut self, config: &Config) {
        if self.refresh.is_some() || self.entries.is_empty() {
            return;
        }
        let ids = self
            .entries
            .iter()
            .map(|entry| entry.execution_id.clone())
            .collect();
        self.refresh = Some(request_refresh(config, ids));
    }

    /// Ask the server to interrupt the selected subtask (no-op in flight).
    pub(crate) fn start_cancel(&mut self, config: &Config) {
        if self.cancel.is_some() {
            return;
        }
        if let Some(entry) = self.selected_entry() {
            self.cancel = Some(request_cancel(config, entry.execution_id.clone()));
        }
    }

    /// Poll the in-flight refresh: `None` while pending, else the result
    /// (slot cleared either way). On success the snapshots merge into the
    /// view AND their status/summary/tokens land in the matching entries.
    pub(crate) fn poll_refresh(&mut self) -> Option<anyhow::Result<()>> {
        let receiver = self.refresh.as_mut()?;
        let snapshots = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(error) => Err(error.into()),
        };
        self.refresh = None;
        match snapshots {
            Ok(snapshots) => {
                for snapshot in snapshots {
                    self.apply_update(
                        &snapshot.execution_id,
                        &snapshot.status,
                        snapshot.summary.as_deref(),
                        snapshot.tokens,
                    );
                    self.snapshots.insert(snapshot.execution_id.clone(), snapshot);
                }
                Some(Ok(()))
            }
            Err(error) => Some(Err(error)),
        }
    }

    /// Poll the in-flight cancel request (`None` while pending).
    pub(crate) fn poll_cancel(&mut self) -> Option<anyhow::Result<()>> {
        let receiver = self.cancel.as_mut()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(error) => Err(error.into()),
        };
        self.cancel = None;
        Some(result)
    }
}

/// Kinds dispatched with a 1:1 local `tui_remote_*` bookmark session created
/// at dispatch time — `Enter` may descend into those conversations.
pub(crate) fn is_conversational(kind: &str) -> bool {
    matches!(kind, "agent" | "operator")
}

/// Result of one key press in the fullscreen view.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SubtaskOutcome {
    Idle,
    Close,
    Append(String),
    EnterConversation(String),
}

/// Route one key press. `term_h` sizes page-scroll steps; `config` arms the
/// `r` (refresh) and `c` (cancel) requests.
pub(crate) fn handle_key(
    view: &mut SubtaskView,
    k: KeyEvent,
    term_h: u16,
    config: &Config,
) -> SubtaskOutcome {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let step = (term_h / 2).max(1) as u32;
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => SubtaskOutcome::Close,
        KeyCode::Char('j') | KeyCode::Down => {
            view.move_selection(1);
            SubtaskOutcome::Idle
        }
        KeyCode::Char('k') | KeyCode::Up => {
            view.move_selection(-1);
            SubtaskOutcome::Idle
        }
        KeyCode::PageUp | KeyCode::Char('u') if ctrl => {
            view.scroll_up(step);
            SubtaskOutcome::Idle
        }
        KeyCode::PageDown | KeyCode::Char('d') if ctrl => {
            view.scroll_down(step);
            SubtaskOutcome::Idle
        }
        KeyCode::Char('r') => {
            view.start_refresh(config);
            SubtaskOutcome::Idle
        }
        KeyCode::Char('c') => {
            view.start_cancel(config);
            SubtaskOutcome::Idle
        }
        // Ctrl+Enter: append the selected subtask's result to the main task.
        KeyCode::Enter if ctrl => match view.selected_entry() {
            Some(entry) => SubtaskOutcome::Append(entry.execution_id.clone()),
            None => SubtaskOutcome::Idle,
        },
        // Enter: descend into the child session (conversational kinds only).
        KeyCode::Enter => match view.selected_entry() {
            Some(entry) if is_conversational(&entry.kind) => {
                SubtaskOutcome::EnterConversation(entry.execution_id.clone())
            }
            _ => SubtaskOutcome::Idle,
        },
        _ => SubtaskOutcome::Idle,
    }
}

/// Map event frames to `"{seq} {kind} {ts}"` rows (ts as unix seconds; the
/// server stores milliseconds). Newest last; at most the LAST `limit` rows
/// survive, prefixed by a `… N earlier events` marker when truncated.
pub(crate) fn timeline_rows(frames: &[Value], limit: usize) -> Vec<String> {
    let mut rows: Vec<String> = frames
        .iter()
        .map(|frame| {
            let seq = frame["seq"].as_i64().unwrap_or(0);
            let kind = frame["kind"].as_str().unwrap_or("event");
            let ts = frame["ts"].as_i64().unwrap_or(0).div_euclid(1000);
            format!("{seq} {kind} {ts}")
        })
        .collect();
    if rows.len() > limit {
        let hidden = rows.len() - limit;
        rows = rows.split_off(hidden);
        rows.insert(0, format!("\u{2026} {hidden} earlier events"));
    }
    rows
}

/// Spawn a snapshot refresh for `ids` on tokio (the one-shot request/poll
/// pattern; the composer loop polls the returned receiver).
pub(crate) fn request_refresh(
    config: &Config,
    ids: Vec<String>,
) -> oneshot::Receiver<anyhow::Result<Vec<SubtaskSnapshot>>> {
    let (tx, rx) = oneshot::channel();
    let config = config.clone();
    tokio::spawn(async move {
        let _ = tx.send(refresh_all(&config, &ids).await);
    });
    rx
}

/// Fetch every id's snapshot; ids that fail (e.g. server 404 after a
/// cleanup) are skipped, but an all-fail refresh surfaces the first error.
async fn refresh_all(config: &Config, ids: &[String]) -> anyhow::Result<Vec<SubtaskSnapshot>> {
    let client = ServerClient::configured(config)?;
    let mut snapshots = Vec::new();
    let mut first_error: Option<anyhow::Error> = None;
    for id in ids {
        match fetch_snapshot(&client, id).await {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    match (snapshots.is_empty(), first_error) {
        (true, Some(error)) => Err(error),
        _ => Ok(snapshots),
    }
}

/// One subtask's full server-side snapshot: status/summary/tokens plus a
/// bounded timeline of its event frames.
async fn fetch_snapshot(client: &ServerClient, id: &str) -> anyhow::Result<SubtaskSnapshot> {
    let status = crate::remote::subtasks::fetch_status(client, id).await?;
    let summary = crate::remote::subtasks::fetch_summary(client, id).await?;
    let tokens = crate::remote::subtasks::fetch_tokens(client, id).await?;
    let frames = paged_frames(client, id).await.unwrap_or_default();
    Ok(SubtaskSnapshot {
        execution_id: id.to_string(),
        status: status.as_str().to_string(),
        summary,
        tokens,
        timeline: timeline_rows(&frames, TIMELINE_MAX_ROWS),
    })
}

/// Page `GET /api/executions/{id}/events-page?after=N` following `more`,
/// bounded to [`TIMELINE_MAX_PAGES`] pages.
async fn paged_frames(client: &ServerClient, id: &str) -> anyhow::Result<Vec<Value>> {
    let mut frames = Vec::new();
    let mut after = 0i64;
    for _ in 0..TIMELINE_MAX_PAGES {
        let page: Value = client
            .get(&format!("/api/executions/{id}/events-page?after={after}"))
            .await?;
        let events = page["events"].as_array().cloned().unwrap_or_default();
        for frame in &events {
            if let Some(seq) = frame["seq"].as_i64() {
                after = after.max(seq);
            }
        }
        let more = page["more"] == Value::Bool(true);
        frames.extend(events);
        if !more {
            break;
        }
    }
    Ok(frames)
}

/// Spawn a server-side interrupt for `id` (one-shot request/poll pattern).
pub(crate) fn request_cancel(config: &Config, id: String) -> oneshot::Receiver<anyhow::Result<()>> {
    let (tx, rx) = oneshot::channel();
    let config = config.clone();
    tokio::spawn(async move {
        let _ = tx.send(send_cancel(&config, &id).await);
    });
    rx
}

async fn send_cancel(config: &Config, id: &str) -> anyhow::Result<()> {
    crate::remote::subtasks::cancel(&ServerClient::configured(config)?, id).await
}

#[cfg(test)]
#[path = "subtask_detail_tests.rs"]
mod tests;
