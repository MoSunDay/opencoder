//! Rebuild local-memory maintenance blocks (`SubagentStart` with
//! `kind == "memory"`) from the PARENT session's persisted event log.
//!
//! A memory run has no child session rows (store-less context copy, by
//! design), so the ordinary subagent reconstruction — `SubagentTaskRecord` +
//! child event log — finds nothing. Its whole durable trace is the wrapped
//! `SubagentChild` frames the parent sink persists. Live, the TUI routes
//! those frames into the foldable memory block; on resume this module walks
//! the same frames and rebuilds the block plus its token spend, so
//! `[tok cost]` keeps the maintenance usage across a restart exactly like
//! the live view.

use std::sync::Arc;

use opencoder_session::SessionEvent;
use opencoder_store::{SessionEventRecord, Store};

use crate::chat::{short, ChatBlock, ChatView};
use crate::terminal_text::sanitize_multiline;

/// One rebuilt memory block plus its temporal anchor: the `ts` of its
/// `SubagentStart` row, used to interleave it after the last message that
/// existed when the run fired.
pub(super) struct MemoryRun {
    pub anchor_ts: i64,
    pub block: ChatBlock,
}

/// Accumulator for the run currently open in the seq walk.
struct OpenRun {
    id: String,
    child_session_id: String,
    prompt: String,
    started_ts: i64,
    last_ts: i64,
    view: ChatView,
}

/// Parse a stored parent-row payload back into a `SessionEvent`. Parent rows
/// are written by the sink/TUI worker in SSE form (`sse_kind` + `sse_data`);
/// `from_stored` also covers the enum-form rows `resume.rs` persists.
pub(super) fn parse_stored_event(rec: &SessionEventRecord) -> Option<SessionEvent> {
    SessionEvent::from_stored(rec)
}

/// Collect the memory runs of a session from its persisted `subagent_*`
/// frames, in seq order. Non-memory subagent frames are ignored (their
/// blocks rebuild from `SubagentTaskRecord` + child rows instead).
pub(super) async fn memory_runs(store: &Arc<dyn Store>, session_id: &str) -> Vec<MemoryRun> {
    let records = store
        .events_of_kinds(
            session_id,
            &["subagent_start", "subagent_child", "subagent_end"],
        )
        .await
        .unwrap_or_default();
    build_memory_runs(&records)
}

/// Pure core of [`memory_runs`]: fold the frames into blocks. Exposed for
/// unit tests (no store round-trip needed).
pub(super) fn build_memory_runs(records: &[SessionEventRecord]) -> Vec<MemoryRun> {
    let mut open: Option<OpenRun> = None;
    let mut done: Vec<MemoryRun> = Vec::new();
    for rec in records {
        let Some(event) = parse_stored_event(rec) else {
            continue;
        };
        match event {
            SessionEvent::SubagentStart {
                id,
                kind,
                prompt,
                child_session_id,
            } => {
                // Memory maintenance never overlaps itself: one run per
                // completed task, sequentially after the parent turn.
                if kind == "memory" && open.is_none() {
                    open = Some(OpenRun {
                        id,
                        child_session_id,
                        prompt,
                        started_ts: rec.ts,
                        last_ts: rec.ts,
                        view: ChatView::default(),
                    });
                }
            }
            SessionEvent::SubagentChild { id, ev } => {
                if let Some(run) = open.as_mut().filter(|r| r.id == id) {
                    run.view.apply(&ev);
                    run.last_ts = run.last_ts.max(rec.ts);
                }
            }
            // Only take on an id match: a task-tool `SubagentEnd` can share
            // the log (take-then-filter would drop the open run).
            SessionEvent::SubagentEnd {
                id,
                ok,
                cancelled,
                summary,
            } if open.as_ref().map(|r| r.id == id).unwrap_or(false) => {
                let run = open.take().expect("id matched an open run");
                done.push(finish_run(run, true, ok, cancelled, summary));
            }
            SessionEvent::SubagentEnd { .. } => {}

            _ => {}
        }
    }
    // Crash-truncated tail: a `SubagentStart` with no `SubagentEnd` displays
    // like an interrupted subagent task (mirrors `build_subagent_block`).
    if let Some(run) = open.take() {
        done.push(finish_run(
            run,
            true,
            false,
            false,
            "(interrupted)".to_string(),
        ));
    }
    done
}

/// Close one accumulated run into a `ChatBlock::Subagent` + anchor pair.
fn finish_run(run: OpenRun, done: bool, ok: bool, cancelled: bool, summary: String) -> MemoryRun {
    let mut view = run.view;
    // A truncated log never emitted round-terminal frames; finalize so an
    // open Say cannot resurrect raw markdown (same repair as
    // `build_subagent_block`).
    view.finalize_assistant();
    MemoryRun {
        anchor_ts: run.started_ts,
        block: ChatBlock::Subagent {
            id: run.id,
            child_session_id: run.child_session_id,
            kind: "memory".into(),
            prompt: short(&run.prompt, 90),
            view,
            done,
            ok,
            cancelled,
            summary: sanitize_multiline(&summary).into_owned(),
            started_at_ms: run.started_ts,
            elapsed_ms: Some((run.last_ts.max(run.started_ts) - run.started_ts).max(0) as u64),
        },
    }
}
