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
    // Publish the live-session pointer immediately so a remote console can
    // attach while the step runs (meta.json's session_id lands on finish).
    crate::step_io::write_session_artifact(
        &ctx.workflow_root,
        &ctx.run_id,
        &ctx.step.name,
        &session_id,
    );
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
    // Workflow-author-declared how.md append: visible to the session's
    // tool processes as OPENCODER_HOW_APPEND, persisted after success.
    let how_append = match &ctx.step.kind {
        StepKind::Agent { how_append, .. } => how_append.clone(),
        _ => None,
    };
    if how_append.is_some() {
        session.env_passthrough = super::how_append::env_pairs(how_append.as_deref());
    }

    // Local durability of the event stream, exactly like a node task. The
    // sink moves into the event callback and drops with it at run end.
    let (sink, flusher) =
        opencoder_session::spawn_event_flusher(Some(deps.store.clone()), session_id.clone());

    let transcript = Arc::new(Mutex::new(String::new()));
    let on_event = {
        let sink = sink;
        let transcript = Arc::clone(&transcript);
        let log = ctx.log.clone();
        move |ev: SessionEvent| {
            let _ = sink.push(&ev);
            if let SessionEvent::TextDelta(text) = &ev {
                if let Ok(mut tail) = transcript.lock() {
                    push_tail(&mut tail, text, MAX_TRANSCRIPT_TAIL);
                }
                if let Some(log) = &log {
                    log.text_delta(text);
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

    // Providers normally stream `TextDelta` frames, but a valid provider (and
    // the deterministic test client) may deliver only a completed message.
    // Recover that persisted assistant text so structured output and the
    // run-scoped step log do not depend on streaming granularity.
    let mut text = transcript.lock().map(|t| t.clone()).unwrap_or_default();
    if text.trim().is_empty() {
        if let Some(completed) = opencoder_session::handoff::last_assistant_text(&session.messages)
        {
            text = completed;
            if let Some(log) = &ctx.log {
                log.text_delta(&text);
            }
        }
    }
    let output_json = extract_output_json_from(&text);
    let (outcome, error) = terminal_step(cancel.is_cancelled(), result.as_ref().err());
    // Successful step: persist the declared how.md append (warn-only — a
    // pool-write failure never flips a successful step to error).
    if outcome == StepOutcome::Done {
        if let Some(delta) = how_append.as_deref().filter(|d| !d.trim().is_empty()) {
            match super::how_append::append_to_how_md(&step_agent_name(&ctx.step), delta) {
                Ok(version) => info!(
                    run_id = %ctx.run_id, step = %ctx.step.name, version,
                    "how_append persisted to agent prompt pool"
                ),
                Err(e) => warn!(
                    run_id = %ctx.run_id, step = %ctx.step.name, error = %e,
                    "how_append persistence failed (step outcome unchanged)"
                ),
            }
        }
    }
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

/// The step's executing agent name — `agent` field or the `act` default,
/// the same resolution `create_session_meta` pins on the session row.
fn step_agent_name(step: &StepSpec) -> String {
    match &step.kind {
        StepKind::Agent { agent, .. } => agent.clone().unwrap_or_else(|| "act".into()),
        _ => "act".into(),
    }
}

/// Prompt = step prompt + upstream context header + structured-output
/// instruction. The context is the same object a wasm step receives as its
/// `context.json` input file (delivered under `/workspace/context`).
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

/// Recover structured output from the final assistant text. Precedence: the
/// LAST ```json fenced block wins (the output contract's primary form); else
/// the last balanced top-level `{...}` object in the reply tail (bare JSON
/// after narration); else the whole trimmed text when it parses as JSON
/// (arrays, scalars); else `None` (the step simply had no structured output).
///
/// Public so the node's agent-kind session executor shares the DAG step's
/// output contract byte for byte.
pub fn extract_output_json_from(text: &str) -> Option<Value> {
    // When a fence exists but its body fails to parse, the bare-JSON scan
    // runs on the text AFTER the fence body: an unclosed `{` inside the dead
    // fence must not swallow the real object that follows it.
    let mut bare_scan = text;
    if let Some(start) = text.rfind("```json") {
        let after = &text[start + "```json".len()..];
        let (body, rest) = match after.find("```") {
            Some(end) => (&after[..end], &after[end + "```".len()..]),
            None => (after, ""),
        };
        bare_scan = rest;
        if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
            return Some(v);
        }
    }
    if let Some(v) = extract_tail_bare_json(bare_scan) {
        return Some(v);
    }
    serde_json::from_str::<Value>(text.trim()).ok()
}

/// How much of the reply tail the bare-JSON fallback scans. Step replies are
/// narration-first, so the final structured object always sits near the end.
const BARE_JSON_TAIL_LIMIT: usize = 8 * 1024;

/// Find every balanced top-level `{...}` span in the tail of `text` and parse
/// the last one that yields valid JSON. Braces inside JSON strings never
/// count (string-aware scan). Byte-level scanning is safe: `{`, `}`, `"`,
/// `\` are ASCII and never occur inside a multi-byte UTF-8 sequence, so span
/// slicing always lands on char boundaries.
fn extract_tail_bare_json(text: &str) -> Option<Value> {
    let start = if text.len() <= BARE_JSON_TAIL_LIMIT {
        0
    } else {
        let mut cut = text.len() - BARE_JSON_TAIL_LIMIT;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        cut
    };
    let bytes = &text.as_bytes()[start..];
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut open_at: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    open_at = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    spans.push((open_at.take().unwrap_or(i), i));
                }
            }
            _ => {}
        }
    }
    spans
        .iter()
        .rev()
        .find_map(|(from, to)| serde_json::from_slice(&bytes[*from..=*to]).ok())
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

    /// Bare JSON after narration (the no-fence contract form): the whole
    /// text is NOT valid JSON, so the pre-fix whole-text parse returned
    /// `None` here — the fallback scan is what recovers the object.
    #[test]
    fn extracts_bare_json_from_narration_tail() {
        let reply = "## 分析过程\n调用链定位到 handler，签名匹配，关键证据如下……（长叙述）\n最终结论：\n{\"depend_type\": \"strong\", \"analysis_report\": {\"调用链\": \"router->handler\"}}\n";
        let value = extract_output_json_from(reply).unwrap();
        assert_eq!(value["depend_type"], serde_json::json!("strong"));
        assert_eq!(
            value["analysis_report"]["调用链"],
            serde_json::json!("router->handler")
        );
        // The LAST top-level object wins when narration carries examples.
        let mixed = "示例 {\"not\": \"this\"} 与 {a: 占位} 说明。\n{\"final\": 7}";
        assert_eq!(
            extract_output_json_from(mixed).unwrap()["final"],
            serde_json::json!(7)
        );
    }

    /// A broken fence falls through to the tail scan instead of dropping the
    /// whole structured output; prose braces never shadow a real object.
    #[test]
    fn broken_fence_falls_back_to_tail_bare_json() {
        // The fence closes, but its body is not valid JSON: the fence branch
        // fails and the tail scan recovers the bare object after it.
        let reply = "```json\n{\"broken\":\n```\n结论：\n{\"depend_type\": \"weak\"}";
        assert_eq!(
            extract_output_json_from(reply).unwrap()["depend_type"],
            serde_json::json!("weak")
        );
        // Braces inside JSON strings are inert; the scan still balances.
        let strings = "{\"code\": \"} { \\n { \"} ... 混入叙述 {\"answer\": 9}";
        assert_eq!(
            extract_output_json_from(strings).unwrap()["answer"],
            serde_json::json!(9)
        );
        // Unterminated object at the very end yields nothing.
        assert!(extract_tail_bare_json("结论 {\"open\": 1").is_none());
    }

    /// The fallback only scans the 8KB tail: objects buried earlier in a
    /// huge reply are out of scope, objects at the end are always reached.
    #[test]
    fn bare_json_scan_is_bounded_to_the_tail() {
        let late = format!("{}\n{{\"tail_only\": true}}", "x".repeat(9000));
        assert!(extract_output_json_from(&late).unwrap()["tail_only"]
            .as_bool()
            .unwrap());
        let early = format!("{{\"head_only\": true}}\n{}", "y".repeat(9000));
        assert!(extract_output_json_from(&early).is_none());
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
