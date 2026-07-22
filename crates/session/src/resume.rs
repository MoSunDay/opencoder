//! Session recovery: reconstruct a `SessionState` from a durable store, and
//! cheap background title generation (uses `small_model`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use opencoder_core::{
    message::now_ms, resolve_agent, Agent, Config, ContentBlock, Message, MessageUsage, Role,
};
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, SessionEventRecord, Store, SubagentStatus, SubagentTaskRecord};

use crate::SessionState;
use tokio_util::sync::CancellationToken;

/// Rebuild a session from persisted history. The agent/model come from the
/// stored session metadata when available, so a resumed session keeps its
/// original configuration rather than the caller's defaults.
pub async fn resume(
    store: Arc<dyn Store>,
    id: &str,
    mut config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
) -> Result<SessionState> {
    let meta = store
        .get_session(id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {id}"))?;

    // Prefer the stored model/agent so resume is faithful to the original run.
    if let Some(m) = &meta.model {
        config.model = m.clone();
    }
    let agent_name = meta.agent.as_deref().unwrap_or(&config.agent.default);
    let agent = resolve_agent(agent_name)
        .or_else(|| resolve_agent("act"))
        .ok_or_else(|| anyhow!("agent not found: {agent_name}"))?;

    let mut messages: Vec<Message> = store.load_messages(id).await?;

    // Reconcile subagent tasks stuck in Running state — the process was
    // interrupted mid-subagent. Mark them Cancelled (not Failed): a cancelled
    // task keeps its parent tool_use open so it is replayed on the next user
    // turn (run_with_registry), rather than recording a terminal error result.
    let tasks = store.list_subagent_tasks(id).await.unwrap_or_default();
    for task in &tasks {
        if task.status == SubagentStatus::Running {
            tracing::warn!(task_id = %task.task_id, "marking stuck Running subagent as Cancelled on resume");
            let _ = store.cancel_subagent_task(&task.task_id).await;
        }
    }

    // Plan→act handoff (dominant reset) and compaction are mutually exclusive
    // on resume: when a handoff boundary was persisted, trim the plan-mode
    // history and re-attach the synthetic plan instruction; otherwise apply a
    // persisted compaction trim. Handoff wins because it replaces the whole
    // transcript, so any stale compaction metadata from plan mode is moot.
    if let Some(hs) = meta.handoff_seq {
        if let Some(plan_display) = &meta.handoff_plan {
            let hs = hs as usize;
            if hs < messages.len() {
                messages = messages[hs..].to_vec();
            } else {
                messages = Vec::new();
            }
            let plan_msg = crate::plan_handoff::handoff_message(plan_display);
            messages.insert(0, plan_msg);
        }
    } else if let Some(skip) = meta.summary_seq {
        if skip > 0 {
            let skip = skip as usize;
            if skip < messages.len() {
                messages = messages[skip..].to_vec();
            } else {
                messages = Vec::new();
            }
        }
        if let Some(summary_text) = &meta.summary {
            let summary_msg = crate::compaction::compaction_message(summary_text.clone());
            messages.insert(0, summary_msg);
        }
    }

    // Reconcile dangling tool_use blocks. If the process was hard-interrupted
    // after the assistant requested tool calls but before the matching
    // tool_result messages were persisted, the transcript holds unmatched
    // `tool_use` ids -- which most OpenAI-compatible providers reject with
    // HTTP 400 on the next call. Synthesize error results for every dangling
    // call, persist them, and append them so history stays well-formed.
    let answered: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    // `task` tool_use ids whose subagent is still in-flight (Running) or was
    // interrupted (Cancelled): these are replayed/backfilled on the next user
    // turn, so leave them dangling rather than synthesizing error results.
    let replayable: HashSet<&str> = tasks
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                SubagentStatus::Running | SubagentStatus::Cancelled
            )
        })
        .map(|t| t.task_id.as_str())
        .collect();
    let dangling: Vec<ContentBlock> = messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. }
                if !answered.contains(id.as_str()) && !replayable.contains(id.as_str()) =>
            {
                Some(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: "session interrupted: tool result missing".to_string(),
                    is_error: true,
                })
            }
            _ => None,
        })
        .collect();
    if !dangling.is_empty() {
        let n_dangling = dangling.len();
        let synthetic = Message {
            id: crate::runner::new_id(),
            role: Role::Tool,
            blocks: dangling,
            model: None,
            agent: None,
            usage: opencoder_core::MessageUsage::default(),
            created_at: opencoder_core::message::now_ms(),
            synthetic: true,
        };
        tracing::warn!(
            session_id = id,
            count = n_dangling,
            "synthesizing error tool_result for dangling tool_use on resume"
        );
        // Persist so a subsequent resume sees a well-formed transcript.
        let _ = store.append_message(id, &synthetic).await;
        messages.push(synthetic);
    }

    let n = messages.len();
    let model = config.model_id().to_string();

    let mut s = SessionState {
        id: id.to_string(),
        messages,
        agent,
        model,
        working_dir,
        config,
        client,
        last_usage: opencoder_llm::Usage::default(),
        store: Some(store),
        skill_prompt: Arc::new(Mutex::new(meta.skill.clone())),
        active_skill_names: Arc::new(Mutex::new(infer_skill_names(&meta.skill))),
        persisted_count: n,
        session_created: true,
        cancel: None,
        summary: meta.summary,
        summary_seq: meta.summary_seq,
        handoff_seq: meta.handoff_seq,
        handoff_plan: meta.handoff_plan.clone(),
    };
    let _ = &mut s;
    Ok(s)
}

/// Replay subagent tasks stuck in `Running` for `id`, then resume the parent.
///
/// When a parent session is hard-interrupted mid-subagent, the task row is
/// left `Running` and the parent's transcript holds an unanswered `task`
/// `tool_use`. This resumes each such child from its persisted transcript,
/// runs it to completion with an empty prompt ("continue"), backfills the
/// resulting `tool_result` into the parent, and marks the task complete.
///
/// Children hold no `task` tool (see `agent.rs`), so a child can never
/// dispatch a grandchild — there is exactly one level and no recursion is
/// needed. The low-level [`resume`] is left untouched: by the time it runs,
/// no task is `Running` and every `task` `tool_use` is answered, so its
/// stuck-task and dangling-`tool_use` reconciliation paths are inert.
pub async fn resume_and_replay(
    store: Arc<dyn Store>,
    id: &str,
    config: Config,
    client: Arc<dyn ChatStream>,
    working_dir: PathBuf,
    replay_cancel: Option<CancellationToken>,
) -> Result<SessionState> {
    let running: Vec<SubagentTaskRecord> = store
        .list_subagent_tasks(id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.status == SubagentStatus::Running)
        .collect();

    // Replay each Running child, collecting results to backfill in ONE Tool
    // message -- mirrors run_loop, which batches a turn's tool results into a
    // single tool message. `list_subagent_tasks` returns rows in `seq` order,
    // so results land deterministically in dispatch order.
    let mut backfill: Vec<ContentBlock> = Vec::with_capacity(running.len());
    for task in &running {
        if let Some(c) = &replay_cancel {
            if c.is_cancelled() {
                break;
            }
        }
        let outcome = replay_child(store.clone(), task, &config, &client, &working_dir).await;
        let (text, ok) = match outcome {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    task_id = %task.task_id,
                    child = %task.child_session_id,
                    error = %e,
                    "subagent replay failed; backfilling an error result"
                );
                (format!("subagent resume failed: {e:#}"), false)
            }
        };
        let _ = store.complete_subagent_task(&task.task_id, &text, ok).await;
        backfill.push(ContentBlock::ToolResult {
            tool_use_id: task.task_id.clone(),
            content: text,
            is_error: !ok,
        });
    }

    // Backfill the tool_results BEFORE resuming, so resume() sees every task
    // `tool_use` as answered and does not synthesize error results for them
    // via its dangling-`tool_use` reconciliation.
    if !backfill.is_empty() {
        let tool_msg = Message {
            id: crate::runner::new_id(),
            role: Role::Tool,
            blocks: backfill,
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        };
        if let Err(e) = store.append_message(id, &tool_msg).await {
            tracing::warn!(
                session_id = id,
                error = %e,
                "failed to backfill replayed tool_results; falling back to plain resume"
            );
        }
    }

    // All tasks are now complete and the task `tool_use` ids are answered, so
    // resume() reconstructs the parent cleanly.
    resume(store, id, config, client, working_dir).await
}

/// Replay subagent tasks left in `Cancelled` status, then mark them complete.
///
/// Called from `run_with_registry` before the main loop runs, so a continued
/// session resumes each cancelled child (run to completion), backfills the
/// resulting `tool_result` into the parent transcript, and flips the task to
/// `Completed`. The model then sees [user input + subagent result] together and
/// the interrupted call is transparently resumed. No-op when there is no store
/// or no cancelled tasks (e.g. children, which hold no `task` tool).
pub async fn replay_cancelled_tasks(session: &mut SessionState) {
    let store = match session.store.clone() {
        Some(s) => s,
        None => return,
    };
    let cancelled: Vec<SubagentTaskRecord> = store
        .list_subagent_tasks(&session.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            t.status == SubagentStatus::Cancelled
                && (session.handoff_seq.is_none()
                    || session.messages.iter().any(|m| {
                        m.blocks.iter().any(
                            |b| matches!(b, ContentBlock::ToolUse { id, .. } if id == &t.task_id),
                        )
                    }))
        })
        .collect();
    if cancelled.is_empty() {
        return;
    }
    // A pending steer means the user explicitly redirected this turn (e.g.
    // clicked the steer-submit button while a subagent was running). Abandon
    // the cancelled subagents instead of replaying them: the user wants to move
    // on, not silently resume the child they just interrupted. Backfill a
    // terminal "cancelled" tool_result so the transcript stays well-formed, and
    // mark each task Failed so it is never replayed again. This unblocks the
    // steer (claimed next in run_loop) without re-running the cancelled child.
    let has_pending_steers = store
        .pending_inputs(&session.id, Delivery::Steer)
        .await
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if has_pending_steers {
        abandon_cancelled_tasks(session, &store, &cancelled).await;
        return;
    }
    let cancel = session.cancel.clone();
    let mut backfill: Vec<ContentBlock> = Vec::with_capacity(cancelled.len());
    for task in &cancelled {
        if let Some(c) = &cancel {
            if c.is_cancelled() {
                break;
            }
        }
        let outcome = replay_child(
            store.clone(),
            task,
            &session.config,
            &session.client,
            &session.working_dir,
        )
        .await;
        let (text, ok) = match outcome {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    task_id = %task.task_id,
                    child = %task.child_session_id,
                    error = %e,
                    "cancelled subagent replay failed; backfilling an error result"
                );
                (format!("subagent resume failed: {e:#}"), false)
            }
        };
        let _ = store.complete_subagent_task(&task.task_id, &text, ok).await;
        backfill.push(ContentBlock::ToolResult {
            tool_use_id: task.task_id.clone(),
            content: text,
            is_error: !ok,
        });
    }
    let tool_msg = Message {
        id: crate::runner::new_id(),
        role: Role::Tool,
        blocks: backfill,
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: now_ms(),
        synthetic: false,
    };
    session.record(tool_msg).await;
}

/// Backfill terminal "cancelled" tool_results for subagent tasks that were
/// interrupted by a user steer (redirect), WITHOUT re-running the children.
/// Each task's `tool_use` gets a terminal error `tool_result` so the transcript
/// stays well-formed (no dangling ids that providers reject with HTTP 400), and
/// the task is marked Failed so `replay_cancelled_tasks` never picks it up
/// again. Used when the user steers to redirect mid-subagent: they want to move
/// on, not resume the interrupted child.
async fn abandon_cancelled_tasks(
    session: &mut SessionState,
    store: &Arc<dyn Store>,
    tasks: &[SubagentTaskRecord],
) {
    const MSG: &str = "cancelled: the user redirected this turn (steer).";
    let mut backfill: Vec<ContentBlock> = Vec::with_capacity(tasks.len());
    for task in tasks {
        let _ = store
            .complete_subagent_task(&task.task_id, MSG, false)
            .await;
        backfill.push(ContentBlock::ToolResult {
            tool_use_id: task.task_id.clone(),
            content: MSG.to_string(),
            is_error: true,
        });
        tracing::info!(
            task_id = %task.task_id,
            child = %task.child_session_id,
            "abandoning cancelled subagent (user steered) instead of replaying"
        );
    }
    let tool_msg = Message {
        id: crate::runner::new_id(),
        role: Role::Tool,
        blocks: backfill,
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: now_ms(),
        synthetic: false,
    };
    session.record(tool_msg).await;
}

/// Resume a single child task and run it to completion with an empty prompt
/// ("continue"). The child's continuation messages and events are persisted to
/// its own session, mirroring `run_subagent`. Returns `(result_text, ok)`.
async fn replay_child(
    store: Arc<dyn Store>,
    task: &SubagentTaskRecord,
    config: &Config,
    client: &Arc<dyn ChatStream>,
    working_dir: &Path,
) -> Result<(String, bool)> {
    // Children never carry subagent tasks of their own (no `task` tool), so
    // resume()'s stuck-task path is a no-op here; its dangling-`tool_use`
    // reconciliation correctly patches a child interrupted mid-tool-call.
    let mut child = resume(
        store.clone(),
        &task.child_session_id,
        config.clone(),
        client.clone(),
        working_dir.to_path_buf(),
    )
    .await?;

    // Incremental child-event persistence (same ordered-flusher pattern as
    // `run_subagent`): events reach the DB as they are produced so a second
    // interruption still leaves partial child progress reconstructable.
    let child_id = task.child_session_id.clone();
    let (ev_tx, ev_rx) = tokio::sync::mpsc::unbounded_channel::<SessionEventRecord>();
    let flush_store = Some(store.clone());
    // Batched, lossless drain (shared with TUI/web/subagent surfaces).
    let flusher = tokio::spawn(crate::event_sink::run_flusher(flush_store, ev_rx));
    let registry = crate::tools::registry();
    // Boxed to break the run_with_registry -> replay_cancelled_tasks ->
    // replay_child -> run_with_registry recursion (children hold no task tool,
    // so replay_cancelled_tasks is a no-op there, but the type must be finite).
    let res = Box::pin(crate::runner::run_with_registry(
        &mut child,
        String::new(),
        &registry,
        move |cev| {
            let rec = SessionEventRecord {
                session_id: child_id.clone(),
                kind: cev.coarse_kind(),
                payload: serde_json::to_value(&cev).unwrap_or(serde_json::Value::Null),
                ts: now_ms(),
                seq: None,
                sse_kind: Some(cev.sse_kind().to_string()),
            };
            if let Err(e) = ev_tx.send(rec) {
                tracing::warn!(error = %e, "replay: child event channel full/closed, dropping event");
            }
        },
    ))
    .await;
    // The callback owned `ev_tx`; once `run_with_registry` returns the closure
    // is dropped, closing the channel so the flusher drains and exits.
    let _ = flusher.await;

    let ok = res.is_ok();
    let text = child
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();
    Ok((text, ok))
}

/// Generate a short title from the first user/assistant exchange, using the
/// small model when configured. Persists the title to the store. Non-fatal:
/// errors are logged and swallowed.
pub async fn generate_title(session: &SessionState) {
    if session.store.is_none() {
        return;
    }
    let store = session.store.clone().unwrap();
    if let Err(e) = generate_title_inner(session, &store).await {
        tracing::warn!(session_id = %session.id, error = %e, "title generation failed");
    }
}

async fn generate_title_inner(session: &SessionState, store: &Arc<dyn Store>) -> Result<()> {
    let msgs = lower_messages(&session.messages);
    let req = ChatRequest {
        model: session.config.small_model_or_primary().to_string(),
        messages: msgs,
        tools: Vec::new(),
        tool_choice: None,
        temperature: Some(0.3),
        max_tokens: Some(64),
        reasoning_effort: None,
        cache_salt: crate::cache_salt_for(session),
    };
    let mut rx = session.client.chat_stream(req).context("title llm call")?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(t) => text.push_str(&t),
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
                break;
            }
            LlmEvent::Error(e) => return Err(anyhow!(e)),
            _ => {}
        }
    }
    let title: String = text.trim().chars().take(80).collect();
    if title.is_empty() {
        return Ok(());
    }
    store
        .update_session(
            &session.id,
            &opencoder_store::SessionPatch {
                title: Some(title),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

#[allow(dead_code)]
fn _ensure_agent_used(_a: &Agent) {}

/// Infer active skill names from a skill prompt body by matching known
/// skill body prefixes. Used on resume to restore latent tool unlocking.
fn infer_skill_names(body: &Option<String>) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut names = HashSet::new();
    if let Some(b) = body {
        let prefix = b.chars().take(200).collect::<String>();
        if prefix.contains("ssh_pty") || prefix.contains("ssh-pty") {
            names.insert("ssh-pty".to_string());
        }
        if prefix.contains("chrome_headless") || prefix.contains("chrome-headless") {
            names.insert("chrome-headless".to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn infer_skill_names_none_body() {
        let names = infer_skill_names(&None);
        assert!(names.is_empty());
    }

    #[test]
    fn infer_skill_names_empty_body() {
        let names = infer_skill_names(&Some(String::new()));
        assert!(names.is_empty());
    }

    #[test]
    fn infer_skill_names_detects_ssh_pty() {
        let body = Some("Use ssh_pty to connect to the server".to_string());
        let names = infer_skill_names(&body);
        assert_eq!(names, HashSet::from(["ssh-pty".to_string()]));
    }

    #[test]
    fn infer_skill_names_detects_ssh_pty_dash() {
        let body = Some("Active skill: ssh-pty".to_string());
        let names = infer_skill_names(&body);
        assert_eq!(names, HashSet::from(["ssh-pty".to_string()]));
    }

    #[test]
    fn infer_skill_names_detects_chrome_headless() {
        let body = Some("Use chrome_headless tool".to_string());
        let names = infer_skill_names(&body);
        assert_eq!(names, HashSet::from(["chrome-headless".to_string()]));
    }

    #[test]
    fn infer_skill_names_detects_both() {
        let body = Some("ssh_pty and chrome-headless are active".to_string());
        let names = infer_skill_names(&body);
        assert_eq!(
            names,
            HashSet::from(["ssh-pty".to_string(), "chrome-headless".to_string()])
        );
    }

    #[test]
    fn infer_skill_names_ignores_after_200_chars() {
        // Content after the first 200 chars should be ignored.
        let padding = "x".repeat(200);
        let body = Some(format!("{padding}ssh_pty"));
        let names = infer_skill_names(&body);
        assert!(
            names.is_empty(),
            "skill names past 200 chars should be ignored"
        );
    }
}
