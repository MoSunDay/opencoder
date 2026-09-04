//! Agent-step executor: run one step's prompt through the REAL session
//! runner on a fresh local session — the same public building blocks the
//! node-task executor composes (`resume_and_replay`, `session.cancel`,
//! `spawn_event_flusher`, `opencoder_session::run`).
//!
//! Cancellation arrives as the step's [`CancellationToken`] and is wired
//! straight into `session.cancel` BEFORE the run, so the runner's own
//! interrupt path converges the turn; no separate flag race is needed.
//! Transcript capture keeps a bounded tail (last ~8KB) and scans it for a
//! ```json fenced block to recover structured output.

use std::sync::{Arc, Mutex};

use opencoder_core::message::now_ms;
use opencoder_dag::{StepKind, StepOutcome, StepSpec};
use opencoder_session::{resume_and_replay as resume_session, run as run_session, SessionEvent};
use opencoder_store::SessionMeta;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{ExecDeps, StepCtx, StepResult};

/// Bounded transcript tail: the artifact/event payload only needs the end of
/// the conversation, never the whole history.
const MAX_TRANSCRIPT_TAIL: usize = 8 * 1024;

/// Execute an `agent` step: one fresh session, one drain, artifacts handled
/// by the caller (the run loop); here we only produce the [`StepResult`].
pub async fn execute_agent_step(
    ctx: &StepCtx,
    deps: &ExecDeps,
    cancel: CancellationToken,
) -> StepResult {
    let session_id = match create_session_meta(deps, &ctx.step, &ctx.run_id).await {
        Ok(id) => id,
        Err(e) => {
            return errored(format!("create session: {e:#}"));
        }
    };
    info!(run_id = %ctx.run_id, step = %ctx.step.name, %session_id, "dag agent step executing");

    // One token doubles as replay guard AND run-loop hard cancel (web parity:
    // the session owns its interrupt path through `session.cancel`).
    let mut session = match resume_session(
        deps.store.clone(),
        &session_id,
        deps.config.clone(),
        deps.client.clone(),
        deps.workdir.clone(),
        Some(cancel.clone()),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return errored(format!("resume session: {e:#}")),
    };
    session.cancel = Some(cancel.clone());
    // Fresh per-step turn token so an interrupt never leaks into later steps.
    session.turn_cancel = Some(Arc::new(Mutex::new(CancellationToken::new())));

    // Local durability of the event stream, exactly like a node task. The
    // sink moves into the event callback and drops with it at run end.
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(deps.store.clone()), session_id.clone());

    let transcript = Arc::new(Mutex::new(String::new()));
    let on_event = {
        let sink = sink;
        let transcript = Arc::clone(&transcript);
        move |ev: SessionEvent| {
            let _ = sink.push(&ev);
            if let SessionEvent::TextDelta(text) = &ev {
                if let Ok(mut tail) = transcript.lock() {
                    push_tail(&mut tail, text, MAX_TRANSCRIPT_TAIL);
                }
            }
        }
    };

    let prompt = build_prompt(ctx);
    let result = run_session(&mut session, prompt, on_event).await;
    // Guarantee the final local flush before reading the transcript.
    if let Err(e) = flusher.await {
        warn!(run_id = %ctx.run_id, step = %ctx.step.name, error = %e, "local event flush failed");
    }

    let text = transcript.lock().map(|t| t.clone()).unwrap_or_default();
    let output_json = extract_output_json_from(&text);
    let (outcome, error) = terminal_step(cancel.is_cancelled(), result.as_ref().err());
    info!(
        run_id = %ctx.run_id,
        step = %ctx.step.name,
        outcome = outcome_str(&outcome),
        "dag agent step finished"
    );
    StepResult {
        outcome,
        error,
        output_text: text,
        output_json,
        session_id: Some(session_id),
    }
}

/// Prompt = step prompt + upstream context header + structured-output
/// instruction. The context is the same object a python step would see as
/// its `context` global.
fn build_prompt(ctx: &StepCtx) -> String {
    let prompt = match &ctx.step.kind {
        StepKind::Agent { prompt, .. } => prompt.clone(),
        _ => String::new(),
    };
    let context = serde_json::to_string_pretty(&ctx.context()).unwrap_or_else(|_| "{}".into());
    format!(
        "{}\n\n上游步骤输出（JSON）：\n{}\n\n如果本步骤需要产出结构化结果，请在最终回复的末尾追加一个 ```json 围栏代码块（fenced code block）包含该 JSON。",
        prompt, context
    )
}

/// Persist a fresh local session row for this step (the node executor's
/// `create_local_meta` shape, but no `task_type` pin: a DAG step session is
/// inspectable like any other).
async fn create_session_meta(
    deps: &ExecDeps,
    step: &StepSpec,
    run_id: &str,
) -> anyhow::Result<String> {
    let (agent, model) = match &step.kind {
        StepKind::Agent { agent, model, .. } => (agent.clone(), model.clone()),
        _ => anyhow::bail!("non-agent step dispatched to the agent executor"),
    };
    let id = ulid::Ulid::new().to_string();
    let now = now_ms();
    deps.store
        .create_session(&SessionMeta {
            id: id.clone(),
            title: Some(format!("dag/{}/{}", run_id, step.name)),
            agent: agent.or_else(|| Some("act".into())),
            model,
            autopilot_mode: None,
            workdir_hash: Some(opencoder_core::workdir_hash(&deps.workdir)),
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await?;
    Ok(id)
}

/// Append `delta`, then trim to the last `max` bytes on a char boundary.
fn push_tail(tail: &mut String, delta: &str, max: usize) {
    tail.push_str(delta);
    if tail.len() > max {
        let mut cut = tail.len() - max;
        while cut < tail.len() && !tail.is_char_boundary(cut) {
            cut += 1;
        }
        let kept = tail[cut..].to_string();
        *tail = kept;
    }
}

/// Recover structured output from the final assistant text: the LAST
/// ```json fenced block wins; else the whole trimmed text when it parses as
/// JSON; else `None` (the step simply had no structured output).
fn extract_output_json_from(text: &str) -> Option<Value> {
    if let Some(start) = text.rfind("```json") {
        let body = &text[start + "```json".len()..];
        let body = body.split("```").next().unwrap_or(body);
        if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
            return Some(v);
        }
    }
    serde_json::from_str::<Value>(text.trim()).ok()
}

/// Terminal decision by precedence: cancelled > error > done (the node
/// executor's `terminal_report`, step-flavored).
fn terminal_step(cancelled: bool, err: Option<&anyhow::Error>) -> (StepOutcome, Option<String>) {
    if cancelled {
        (StepOutcome::Cancelled, None)
    } else {
        match err {
            Some(e) => (StepOutcome::Error, Some(format!("{e:#}"))),
            None => (StepOutcome::Done, None),
        }
    }
}

fn errored(msg: String) -> StepResult {
    StepResult {
        outcome: StepOutcome::Error,
        error: Some(msg),
        output_text: String::new(),
        output_json: None,
        session_id: None,
    }
}

fn outcome_str(o: &StepOutcome) -> &'static str {
    match o {
        StepOutcome::Done => "done",
        StepOutcome::Error => "error",
        StepOutcome::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The json-fence scanner: last fence wins, prose around it is ignored,
    /// a bare JSON text still parses, garbage stays `None`.
    #[test]
    fn extracts_json_from_fence_or_whole_text() {
        let fenced = "analysis...\n```json\n{\"answer\": 1}\n```\ntail";
        assert_eq!(
            extract_output_json_from(fenced).unwrap()["answer"],
            serde_json::json!(1)
        );
        let two_fences = "```json\n{\"first\": true}\n```\nmore\n```json\n{\"second\": 2}\n```";
        assert_eq!(
            extract_output_json_from(two_fences).unwrap()["second"],
            serde_json::json!(2)
        );
        assert_eq!(
            extract_output_json_from("  {\"bare\": 3}  ").unwrap()["bare"],
            serde_json::json!(3)
        );
        assert!(extract_output_json_from("no structure here").is_none());
        // An unterminated fence still yields its body.
        assert_eq!(
            extract_output_json_from("```json\n{\"open\": 4}").unwrap()["open"],
            serde_json::json!(4)
        );
    }

    /// The transcript tail keeps the LAST bytes on a char boundary.
    #[test]
    fn transcript_tail_is_bounded_and_char_safe() {
        let mut t = String::new();
        push_tail(&mut t, &"ab".repeat(100), 16);
        assert_eq!(t.len(), 16);
        push_tail(&mut t, "é", 16); // multi-byte at the seam must not panic
        assert!(t.len() >= 16 && t.len() <= 18);
        assert!(t.chars().last().is_some());
    }
}
