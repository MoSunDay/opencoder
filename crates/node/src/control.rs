//! Worker-side P3 control-plane: serve `fetch_messages` control tasks.
//!
//! The server relays a browser's "open this dialog" request to the node that
//! holds the conversation. This module is the node's half:
//!
//! * [`select_resume_slice`] — the PURE boundary rule. It mirrors the TUI
//!   replay / `session::resume` compaction semantics exactly: when the
//!   session carries a compaction summary with a positive `summary_seq`, the
//!   slice is that summary PLUS only the rows AFTER it; without a summary the
//!   full transcript is the slice.
//! * [`execute_fetch`] — the IO wrapper: read session meta + raw message rows
//!   from the LOCAL store, apply the selector, and wrap every failure into an
//!   `ok:false` result (a missing dialog is a reportable outcome, never a
//!   crash of the runner loop).
//! * [`handle_control`] — one control task end-to-end: dedup by `control_id`,
//!   execute, upload via [`crate::uplink::Uplink::post_control_result`].
//!   Shared by the idle claim arm AND both heartbeat loops (a busy worker is
//!   only reachable through its heartbeat).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use opencoder_core::node_protocol::{ControlTask, DialogMessage, FetchMessagesResult};
use opencoder_store::{MessageRow, Store};
use tracing::{info, warn};

use crate::uplink::Uplink;

/// Input of the pure slice selector: the session's summary pair plus its raw
/// message rows (already ordered by `seq` ASC).
pub struct SliceInput {
    pub summary: Option<String>,
    pub summary_seq: Option<i64>,
    pub rows: Vec<Row>,
}

/// One raw message row (see `opencoder_store::MessageRow`); kept separate from
/// the wire DTO so the selector has zero protocol dependencies.
pub struct Row {
    pub seq: i64,
    pub role: String,
    pub blocks: serde_json::Value,
    pub created_at: i64,
}

impl From<Row> for DialogMessage {
    fn from(r: Row) -> Self {
        DialogMessage {
            seq: r.seq,
            role: r.role,
            blocks: r.blocks,
            created_at: r.created_at,
        }
    }
}

/// Resume-shaped slice boundary — a pure mirror of the resume semantics in
/// `crates/tui/src/session_ui/replay.rs` / `session::resume`:
///
/// * no summary (or a non-positive `summary_seq`): keep ALL rows;
/// * summary with `summary_seq > 0`: keep the summary pair unchanged and only
///   rows with `seq > summary_seq` (the compacted head is represented BY the
///   summary and must not repeat as raw rows).
///
/// Returns `(summary, summary_seq, kept_rows)` in input order.
pub fn select_resume_slice(input: SliceInput) -> (Option<String>, Option<i64>, Vec<Row>) {
    let boundary = match input.summary_seq {
        // No summary / degenerate boundary => full transcript.
        Some(sk) if sk > 0 => sk,
        _ => {
            return (input.summary, input.summary_seq, input.rows);
        }
    };
    let kept: Vec<Row> = input
        .rows
        .into_iter()
        .filter(|r| r.seq > boundary)
        .collect();
    (input.summary, input.summary_seq, kept)
}

/// Execute one `fetch_messages` control task against the local store. Errors
/// are DATA (`ok:false` + reason), so the browser always learns the outcome
/// through the relay instead of the request silently timing out.
pub async fn execute_fetch(store: Arc<dyn Store>, task: &ControlTask) -> FetchMessagesResult {
    let fail = |error: String| FetchMessagesResult {
        control_id: task.control_id.clone(),
        session_id: task.session_id.clone(),
        ok: false,
        error: Some(error),
        summary: None,
        summary_seq: None,
        messages: Vec::new(),
    };
    let meta = match store.get_session(&task.session_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return fail(format!(
                "session {} not found on this node",
                task.session_id
            ));
        }
        Err(e) => return fail(format!("get_session: {e:#}")),
    };
    let rows: Vec<Row> = match store.load_message_rows(&task.session_id).await {
        Ok(rows) => rows.into_iter().map(row_from_store).collect(),
        Err(e) => return fail(format!("load_message_rows: {e:#}")),
    };
    let (summary, summary_seq, kept) = select_resume_slice(SliceInput {
        summary: meta.summary.clone(),
        summary_seq: meta.summary_seq,
        rows,
    });
    FetchMessagesResult {
        control_id: task.control_id.clone(),
        session_id: task.session_id.clone(),
        ok: true,
        error: None,
        summary,
        summary_seq,
        messages: kept.into_iter().map(DialogMessage::from).collect(),
    }
}

fn row_from_store(r: MessageRow) -> Row {
    Row {
        seq: r.seq,
        role: r.role,
        blocks: r.blocks,
        created_at: r.created_at,
    }
}

/// `control_id` dedup guard shared by the claim arm and both heartbeat loops:
/// the same control task can arrive twice within milliseconds (claim reply
/// racing the heartbeat batch); the first wins. Cheap to clone so the main
/// loop and every per-task heartbeater share one set.
#[derive(Clone, Default)]
pub struct Inflight {
    ids: Arc<Mutex<HashSet<String>>>,
}

impl Inflight {
    pub fn new() -> Self {
        Inflight::default()
    }

    /// Claim `id`; `false` means it is already being served.
    pub fn insert_if_absent(&self, id: &str) -> bool {
        match self.ids.lock() {
            Ok(mut set) => set.insert(id.to_string()),
            Err(_) => false,
        }
    }

    pub fn remove(&self, id: &str) {
        if let Ok(mut set) = self.ids.lock() {
            set.remove(id);
        }
    }
}

/// Serve ONE control task: dedup gate -> local fetch -> result upload. The
/// in-flight entry is released only after the upload attempt so a slow upload
/// also suppresses duplicate deliveries; a re-delivery after completion is
/// safe (the read is idempotent, the server answers `resolved:false`).
pub async fn handle_control(
    uplink: &Uplink,
    store: &Arc<dyn Store>,
    inflight: &Inflight,
    node_id: &str,
    task: &ControlTask,
) {
    if !task
        .kind
        .eq_ignore_ascii_case(opencoder_core::node_protocol::TASK_KIND_FETCH_MESSAGES)
    {
        warn!(control_id = %task.control_id, kind = %task.kind, "unsupported control kind; ignored");
        return;
    }
    if !inflight.insert_if_absent(&task.control_id) {
        info!(control_id = %task.control_id, "duplicate control task; already in flight");
        return;
    }
    let result = execute_fetch(Arc::clone(store), task).await;
    if let Err(e) = uplink.post_control_result(node_id, &result).await {
        warn!(control_id = %task.control_id, error = %e, "control result upload failed");
    }
    inflight.remove(&task.control_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: i64) -> Row {
        Row {
            seq,
            role: "user".into(),
            blocks: serde_json::json!([{ "kind": "text", "text": format!("m{seq}") }]),
            created_at: 1000 + seq,
        }
    }

    /// No summary -> the full transcript is the slice (summary stays None).
    #[test]
    fn no_summary_keeps_all_rows() {
        let (summary, summary_seq, kept) = select_resume_slice(SliceInput {
            summary: None,
            summary_seq: None,
            rows: vec![row(1), row(2), row(3)],
        });
        assert!(summary.is_none() && summary_seq.is_none());
        assert_eq!(kept.iter().map(|r| r.seq).collect::<Vec<_>>(), [1, 2, 3]);
    }

    /// With a summary the boundary is strict: only `seq > summary_seq`.
    #[test]
    fn summary_seq_keeps_only_later_rows() {
        let (summary, summary_seq, kept) = select_resume_slice(SliceInput {
            summary: Some("earlier".into()),
            summary_seq: Some(2),
            rows: vec![row(1), row(2), row(3), row(4)],
        });
        assert_eq!(summary.as_deref(), Some("earlier"));
        assert_eq!(summary_seq, Some(2));
        assert_eq!(kept.iter().map(|r| r.seq).collect::<Vec<_>>(), [3, 4]);
    }

    /// Degenerate boundary (`<= 0`) means "not compacted": all rows.
    #[test]
    fn non_positive_summary_seq_keeps_all_rows() {
        for sk in [0, -3] {
            let (_, _, kept) = select_resume_slice(SliceInput {
                summary: Some("s".into()),
                summary_seq: Some(sk),
                rows: vec![row(1), row(2)],
            });
            assert_eq!(kept.len(), 2, "summary_seq={sk} must not trim");
        }
    }

    /// Summary without a seq is passed through unchanged with ALL rows
    /// (matching resume, which only trims on a usable boundary).
    #[test]
    fn summary_without_seq_keeps_all_rows() {
        let (summary, summary_seq, kept) = select_resume_slice(SliceInput {
            summary: Some("only summary".into()),
            summary_seq: None,
            rows: vec![row(1)],
        });
        assert_eq!(summary.as_deref(), Some("only summary"));
        assert!(summary_seq.is_none());
        assert_eq!(kept.len(), 1);
    }

    /// Empty transcript + summary: the slice is just the summary pair.
    #[test]
    fn empty_rows_yield_empty_slice() {
        let (summary, summary_seq, kept) = select_resume_slice(SliceInput {
            summary: Some("s".into()),
            summary_seq: Some(9),
            rows: vec![],
        });
        assert_eq!(summary.as_deref(), Some("s"));
        assert_eq!(summary_seq, Some(9));
        assert!(kept.is_empty());
    }

    /// Row -> wire DTO conversion is lossless.
    #[test]
    fn row_converts_to_dialog_message() {
        let r = row(7);
        let d: DialogMessage = r.into();
        assert_eq!(d.seq, 7);
        assert_eq!(d.role, "user");
        assert_eq!(d.blocks[0]["text"], "m7");
        assert_eq!(d.created_at, 1007);
    }

    /// Dedup guard: first insert wins, removal re-arms.
    #[test]
    fn inflight_dedups_by_control_id() {
        let inflight = Inflight::new();
        assert!(inflight.insert_if_absent("c1"));
        assert!(!inflight.insert_if_absent("c1"), "second claim is a dup");
        assert!(inflight.insert_if_absent("c2"));
        inflight.remove("c1");
        assert!(inflight.insert_if_absent("c1"), "released id re-arms");
    }
}
