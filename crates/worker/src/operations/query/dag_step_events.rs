//! DAG single-step event stream: the query behind
//! `GET /api/dag/runs/:id/steps/:step/events`. An `agent` step owns a child
//! session (`session.json`, falling back to `meta.json.session_id`) and its
//! events are read straight from it; `wasm` steps multiplex into the
//! run session, so their frames are selected by `payload.step` (wasm output
//! additionally by the `step_output` kind). Frame shape, page caps and the
//! 413 guards match `query::events` so the control SSE loop is reusable.

use super::dag_steps::{outcome_status, step_meta, step_session_id};
use super::view::*;
use crate::Worker;
use anyhow::Result;
use opencoder_core::{fleet::*, message::now_ms};
use opencoder_store::SessionEventRecord;
use serde_json::{json, Value};

/// Run-session pages scanned while filtering down to one step. Bounded so a
/// step without matching events cannot make one poll replay the whole run;
/// `more` then stays truthful and the next poll continues from the cursor.
const MAX_FILTER_PAGES: usize = 8;
/// Reserved for the reply envelope (`step` view + terminal frame) so the whole
/// body stays inside one protocol frame.
const ENVELOPE_RESERVE: usize = 128 * 1024;

pub(in crate::operations) async fn dag_step_events(
    worker: &Worker,
    execution: &ExecutionRef,
    step: &str,
    after: i64,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if execution.kind != ExecutionKind::Dag {
        return Ok(RpcReply::error(
            400,
            "dag step events require a DAG execution",
        ));
    }
    let (definition, legacy) = {
        let journal = worker.inner.journal.lock().await;
        let record = journal
            .records
            .get(&execution.id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("dag execution not found"))?;
        (
            record.assignment.definition,
            journal.uses_legacy(&execution.id),
        )
    };
    if !spec_step_names(definition.as_ref())
        .iter()
        .any(|name| name == step)
    {
        return Ok(RpcReply::error(404, "step not found in run spec"));
    }
    let kind = spec_step_kind(definition.as_ref(), step);
    let root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
    };
    let meta = step_meta(&root, &execution.id, step).await?;
    let status = outcome_status(&meta);
    let session_id = step_session_id(&root, &execution.id, step, &meta).await?;
    // An agent step streams its own child session once it exists; until then
    // (and for wasm) the run session is the only source.
    let child = kind.as_deref() == Some("agent") && session_id.is_some();
    let source = if child {
        session_id.clone().unwrap_or_default()
    } else {
        execution.id.clone()
    };
    let filter = (!child).then_some(StepFilter {
        step,
        kind: kind.as_deref(),
    });

    let store = &worker.inner.state.store;
    let mut frames = Vec::new();
    let mut more = false;
    let mut head_seq = 0i64;
    // A missing (not yet created) session is not an error: the step view still
    // answers, and the stream keeps polling instead of terminating early.
    if store.get_session(&source).await?.is_some() {
        head_seq = store.last_event_seq(&source).await?;
        let mut cursor = after;
        for _ in 0..MAX_FILTER_PAGES {
            let page = match store
                .events_page(
                    &source,
                    cursor,
                    EVENT_PAGE_MAX,
                    QUERY_RESPONSE_BYTES - 64 * 1024,
                )
                .await
            {
                Ok(page) => page,
                Err(error)
                    if error
                        .to_string()
                        .contains("exceeds the event page byte limit") =>
                {
                    return Ok(RpcReply::error(413, error.to_string()));
                }
                Err(error) => return Err(error),
            };
            more = page.more;
            let watermark = page.events.iter().filter_map(|event| event.seq).max();
            for event in page.events {
                let Some(seq) = event.seq else { continue };
                if filter.is_some_and(|filter| !filter.matches(&event)) {
                    continue;
                }
                frames.push(json!({
                    "seq": seq,
                    "kind": event.sse_kind.clone().unwrap_or_else(|| "status".into()),
                    "data": event.payload,
                    "ts": event.ts,
                }));
            }
            // Keep the first page that yielded frames; a fully filtered page
            // advances the scan cursor so the poll makes progress.
            if !frames.is_empty() || !more {
                break;
            }
            cursor = watermark.unwrap_or(cursor);
        }
    }

    let mut bytes = 0usize;
    let mut count = 0usize;
    for frame in &frames {
        let size = serde_json::to_vec(frame)?.len();
        if count == 200 || bytes + size > MAX_FRAME_BYTES - ENVELOPE_RESERVE {
            break;
        }
        bytes += size;
        count += 1;
    }
    if count == 0 && !frames.is_empty() {
        return Ok(RpcReply::error(
            413,
            "event exceeds the control frame limit",
        ));
    }
    more = more || frames.len() > count;
    frames.truncate(count);

    // The step receipt, not the run's `active` map, decides termination: an
    // agent step's child session is never registered as an active execution.
    let finished = !more && matches!(status, "done" | "error" | "cancelled");
    if finished {
        let base = frames
            .iter()
            .filter_map(|frame| frame["seq"].as_i64())
            .max()
            .unwrap_or(after)
            .max(0);
        frames.push(json!({
            "seq": base + 1,
            "kind": "step_finished",
            "data": {
                "status": status,
                "error": step_error(&meta),
                "started_at_ms": meta["started_at_ms"],
                "finished_at_ms": meta["finished_at_ms"],
            },
            "ts": now_ms(),
        }));
    }

    let mut step_view = json!({
        "name": step,
        "kind": kind,
        "status": status,
        "error": step_error(&meta),
        "started_at_ms": meta["started_at_ms"],
        "finished_at_ms": meta["finished_at_ms"],
    });
    if let Some(session_id) = &session_id {
        step_view["session_id"] = json!(session_id);
    }
    Ok(RpcReply::ok(json!({
        "events": frames,
        "more": more,
        "finished": finished,
        "head_seq": head_seq,
        "step": step_view,
    })))
}

/// Run-session frames belong to a step only when the payload names it; wasm
/// output is additionally restricted to the `step_output` kind so a step never
/// inherits another step's stdout/stderr.
#[derive(Clone, Copy)]
struct StepFilter<'a> {
    step: &'a str,
    kind: Option<&'a str>,
}

impl StepFilter<'_> {
    fn matches(&self, event: &SessionEventRecord) -> bool {
        if self.kind == Some("wasm")
            && !matches!(event.sse_kind.as_deref(), Some("step_output" | "step_log"))
        {
            return false;
        }
        event.payload["step"].as_str() == Some(self.step)
    }
}

/// Step error text, bounded so a long failure cannot crowd out the events.
fn step_error(meta: &Value) -> Value {
    match meta["error"].as_str() {
        Some(text) => json!(truncate_error_ref(text)),
        None => meta["error"].clone(),
    }
}
