//! Queueable control commands: `/act`, `/plan`, `/act_clear_context`.
//!
//! These slash commands switch the runtime agent (mode) and/or clear the
//! transcript. Unlike normal prompts, they take effect *immediately* when
//! consumed by the drain loop — they do NOT consume an LLM turn. This lets
//! them be interleaved with real prompts/skills in the queue:
//!
//! ```text
//! queue: [/plan] -> [review skill] -> [/act]
//! drain: switch->plan (no turn) . run "review" in plan . switch->act (no turn)
//! ```
//!
//! Integration points (all in [`crate::runner`]):
//! - **Idle short-circuit** ([`run_with_registry`]): when the idle prompt is a
//!   control command, apply it and return without entering [`run_loop`].
//! - **Queue intercept** (run_loop idle boundary): drain consecutive control
//!   commands without an LLM turn until a real prompt arrives.
//! - **Steer intercept** (run_loop turn boundary, defensive): a steered control
//!   command is applied immediately instead of being recorded as user text.

use anyhow::Result;
use opencoder_core::{message::now_ms, resolve_agent, ContentBlock, Message};
use opencoder_store::SessionPatch;

use crate::runner::new_id;
use crate::runner::SessionEvent;
use crate::SessionState;

/// Sentinel value stored in `handoff_plan` so [`crate::resume`] reconstructs a
/// fresh-start marker (not a plan->act handoff instruction) after a
/// [`ControlCmd::ClearContext`]. The distinctive ASCII framing guarantees it
/// never collides with real plan text (no LLM/user output starts with this).
pub(crate) const CLEAR_CONTEXT_SENTINEL: &str = "<<OPENCODER_CLEAR_CONTEXT_MARKER>>";

/// True when a persisted `handoff_plan` is the clear-context sentinel — i.e.
/// the boundary was written by [`ControlCmd::ClearContext`], not a plan->act
/// handoff. Public so display layers (TUI plan card, CLI JSON dump) can skip
/// the raw sentinel instead of ever outputting it; the LLM must never see it
/// (resume converts it to [`fresh_start_message`] before rebuilding context).
pub fn is_clear_context_handoff(handoff_plan: &str) -> bool {
    handoff_plan == CLEAR_CONTEXT_SENTINEL
}

/// Body of the fresh-start marker message left after a context clear.
const CLEAR_CONTEXT_BODY: &str = "[Context cleared - starting fresh in act mode.]";

/// A control command parsed from a slash-command string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlCmd {
    /// Switch the active agent (mode) without resetting context.
    SwitchAgent(String),
    /// Clear the transcript to a single fresh-start marker and switch to act.
    ClearContext,
}

/// Split a compound prompt into its leading control command and any trailing
/// argument text. Supports `/plan <args>` and `/act <args>` so a single
/// submission like `/plan review` switches mode *and* runs the rest as a
/// prompt in the new mode.
///
/// `/act_clear_context` is a sentinel that accepts NO arguments — it matches
/// only on its own (trailing text must not be absorbed as a prompt).
///
/// Returns `None` for anything that is not a control command. The rest text is
/// the trimmed remainder after the command token, or `None` when the input was
/// a bare command with nothing following (e.g. `/plan`). Inline `$skill`
/// tokens in the rest are preserved verbatim for downstream resolution.
pub fn split_control_prefix(prompt: &str) -> Option<(ControlCmd, Option<String>)> {
    let trimmed = prompt.trim();
    // Sentinel: exact match only — never takes an argument.
    if trimmed == "/act_clear_context" {
        return Some((ControlCmd::ClearContext, None));
    }
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?;
    let cmd = match head {
        "/act" => ControlCmd::SwitchAgent("act".into()),
        "/plan" => ControlCmd::SwitchAgent("plan".into()),
        _ => return None,
    };
    let rest: String = parts.collect::<Vec<_>>().join(" ");
    let rest = (!rest.is_empty()).then_some(rest);
    Some((cmd, rest))
}

/// Parse a user prompt into a control command. Returns `None` for anything that
/// is not `/act`, `/plan`, or `/act_clear_context` (the first two accept an
/// optional trailing argument). Compound inputs like `/plan review` are now
/// recognized as a control command; use [`split_control_prefix`] to also
/// recover the trailing argument.
pub fn parse(prompt: &str) -> Option<ControlCmd> {
    split_control_prefix(prompt).map(|(cmd, _)| cmd)
}

/// Apply a control command to `session`, mutating state, persisting via the
/// store, and emitting the appropriate [`SessionEvent`]s so the UI updates.
/// Never calls the LLM.
pub async fn apply(
    session: &mut SessionState,
    cmd: &ControlCmd,
    on_event: &mut (impl FnMut(SessionEvent) + ?Sized),
) -> Result<()> {
    match cmd {
        ControlCmd::SwitchAgent(name) => {
            if let Some(a) = resolve_agent(name) {
                session.agent = a;
                if name == "plan" {
                    session.plan_input_count = 0;
                }
                persist_agent(session, name).await;
                on_event(SessionEvent::AgentSwitch(name.clone()));
            }
        }
        ControlCmd::ClearContext => {
            // Preserve the finalized plan via plan->act handoff when one exists;
            // fall back to a blank fresh-start only when no plan was produced.
            let plan_display = crate::plan_handoff::handoff(session, "");

            if plan_display.is_none() {
                // No plan to carry forward: blank fresh-start sentinel path.
                // Total store messages that predate the clear (the history to
                // trim on resume). Accounts for any in-memory-only summary.
                let store_msg_count = session.store_message_count();
                let preserved_images = crate::compaction::collect_head_images(&session.messages);
                let mut marker = fresh_start_message();
                for url in &preserved_images {
                    marker.blocks.push(ContentBlock::Image {
                        url: url.clone(),
                        detail: None,
                    });
                }
                session.messages = vec![marker];
                // Record the boundary so resume reconstructs the fresh marker,
                // not the full cleared history.
                session.after_handoff(store_msg_count as i64, CLEAR_CONTEXT_SENTINEL.to_string());
            }

            // Clear context always switches to act.
            if let Some(a) = resolve_agent("act") {
                session.agent = a;
            }
            session.set_skill(None);

            persist_clear(session).await;
            on_event(SessionEvent::AgentSwitch("act".into()));
            on_event(SessionEvent::TranscriptReset(session.messages.clone()));
            // When a plan was handed off, surface it so the display layer can
            // render a read-only plan card (mirrors the TUI worker path).
            if let Some(plan) = plan_display {
                on_event(SessionEvent::PlanHandoff(plan));
            }
        }
    }
    Ok(())
}

/// Build the synthetic fresh-start marker message. Exposed so [`crate::resume`]
/// can reconstruct the exact same message on resume.
pub fn fresh_start_message() -> Message {
    let mut msg = Message::user(new_id(), CLEAR_CONTEXT_BODY);
    msg.synthetic = true;
    msg
}

/// Persist an agent switch to the store. Closes the latent
/// resume-persistence gap where a mode switch via `UiCmd::SwitchAgent` (TUI
/// key handler) was not durably recorded: the worker now calls this so
/// `resume()` and the `/task` picker read the switched mode.
pub async fn persist_agent(session: &SessionState, agent: &str) {
    if let Some(store) = &session.store {
        let _ = store
            .update_session(
                &session.id,
                &SessionPatch {
                    agent: Some(agent.into()),
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await;
    }
}

/// Persist the clear-context boundary: handoff metadata + agent = act.
async fn persist_clear(session: &SessionState) {
    if let Some(store) = &session.store {
        let _ = store
            .update_session(
                &session.id,
                &SessionPatch {
                    agent: Some("act".into()),
                    handoff_seq: session.handoff_seq,
                    handoff_plan: session.handoff_plan.clone(),
                    clear_summary: true,
                    clear_skill: true,
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config, ContentBlock};
    use opencoder_llm::{ChatStream, MockChatClient};
    use opencoder_store::LibsqlStore;

    fn make_session(store: Option<Arc<dyn opencoder_store::Store>>) -> SessionState {
        let working_dir = std::env::temp_dir().join("opencoder-control-cmd-tests");
        let mut s = SessionState::new(
            "sess-ctrl",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            working_dir,
        );
        if let Some(st) = store {
            s = s.with_store(st).mark_session_created();
        }
        s
    }

    #[test]
    fn clear_context_sentinel_predicate() {
        assert!(is_clear_context_handoff(CLEAR_CONTEXT_SENTINEL));
        assert!(!is_clear_context_handoff("## Plan\n1. do X"));
        assert!(!is_clear_context_handoff(""));
    }

    #[test]
    fn parse_exact_matches() {
        assert_eq!(parse("/act"), Some(ControlCmd::SwitchAgent("act".into())));
        assert_eq!(parse("/plan"), Some(ControlCmd::SwitchAgent("plan".into())));
        assert_eq!(parse("/act_clear_context"), Some(ControlCmd::ClearContext));
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(
            parse("  /plan  "),
            Some(ControlCmd::SwitchAgent("plan".into()))
        );
        assert_eq!(
            parse("\t/act\n"),
            Some(ControlCmd::SwitchAgent("act".into()))
        );
    }

    #[test]
    fn parse_rejects_non_matches() {
        assert_eq!(parse("/act "), Some(ControlCmd::SwitchAgent("act".into()))); // trailing ws ok
        assert_eq!(parse("/acting"), None);
        assert_eq!(parse("/act_clear"), None);
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("/compact"), None);
    }

    #[test]
    fn split_compound_plan_returns_rest() {
        let (cmd, rest) = split_control_prefix("/plan review the code").unwrap();
        assert_eq!(cmd, ControlCmd::SwitchAgent("plan".into()));
        assert_eq!(rest.as_deref(), Some("review the code"));
    }

    #[test]
    fn split_compound_act_returns_rest() {
        let (cmd, rest) = split_control_prefix("/act do thing").unwrap();
        assert_eq!(cmd, ControlCmd::SwitchAgent("act".into()));
        assert_eq!(rest.as_deref(), Some("do thing"));
    }

    #[test]
    fn split_bare_command_returns_none_rest() {
        let (cmd, rest) = split_control_prefix("/plan").unwrap();
        assert_eq!(cmd, ControlCmd::SwitchAgent("plan".into()));
        assert!(rest.is_none(), "bare command has no rest");
    }

    #[test]
    fn split_trims_whitespace_no_rest() {
        let (cmd, rest) = split_control_prefix("  /act  ").unwrap();
        assert_eq!(cmd, ControlCmd::SwitchAgent("act".into()));
        assert!(rest.is_none());
    }

    #[test]
    fn split_clear_context_takes_no_args() {
        let (cmd, rest) = split_control_prefix("/act_clear_context").unwrap();
        assert_eq!(cmd, ControlCmd::ClearContext);
        assert!(rest.is_none());
    }

    #[test]
    fn split_clear_context_with_args_not_recognized() {
        // A compound "/act_clear_context review" is NOT a control command —
        // the sentinel does not take arguments — so it is treated as text.
        assert_eq!(split_control_prefix("/act_clear_context review"), None);
    }

    #[test]
    fn split_rejects_non_commands() {
        assert_eq!(split_control_prefix("/acting"), None);
        assert_eq!(split_control_prefix("/act_clear"), None);
        assert_eq!(split_control_prefix("hello world"), None);
        assert_eq!(split_control_prefix(""), None);
        assert_eq!(split_control_prefix("/compact"), None);
    }

    #[test]
    fn split_preserves_dollar_tokens_in_rest() {
        // `$skill` tokens survive in the rest for downstream skill resolution.
        let (cmd, rest) = split_control_prefix("/plan $review do it").unwrap();
        assert_eq!(cmd, ControlCmd::SwitchAgent("plan".into()));
        assert_eq!(rest.as_deref(), Some("$review do it"));
    }

    #[test]
    fn parse_compound_recognized_as_command() {
        // The fix: parse now returns the command for compound inputs.
        assert_eq!(
            parse("/plan review"),
            Some(ControlCmd::SwitchAgent("plan".into()))
        );
    }

    fn collect_events(session: &mut SessionState, cmd: ControlCmd) -> Vec<SessionEvent> {
        let mut evs = Vec::new();
        let mut on_event = |ev: SessionEvent| evs.push(ev);
        let _ = futures::executor::block_on(apply(session, &cmd, &mut on_event));
        evs
    }

    #[tokio::test]
    async fn apply_switch_agent_changes_agent_and_emits() {
        let store =
            Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn opencoder_store::Store>;
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "sess-ctrl".into(),
                agent: Some("act".into()),
                created_at: 0,
                updated_at: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        let mut session = make_session(Some(store.clone()));

        let evs = collect_events(&mut session, ControlCmd::SwitchAgent("plan".into()));
        assert_eq!(session.agent.name, "plan");
        assert!(evs
            .iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")));

        // Persisted to the store.
        let meta = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(meta.agent.as_deref(), Some("plan"));
    }

    #[tokio::test]
    async fn apply_clear_context_collapses_and_emits() {
        // A finalized plan exists -> ClearContext preserves it via plan->act
        // handoff rather than wiping to a blank fresh-start.
        let store =
            Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn opencoder_store::Store>;
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "sess-ctrl".into(),
                agent: Some("plan".into()),
                created_at: 0,
                updated_at: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        let mut session = make_session(Some(store.clone()));
        // Start in plan mode with some history.
        session.agent = resolve_agent("plan").unwrap();
        session.messages.push(Message::user("u1", "hello"));
        let mut a = Message::assistant("a1");
        a.blocks.push(ContentBlock::text("plan text"));
        session.messages.push(a);

        let evs = collect_events(&mut session, ControlCmd::ClearContext);

        assert_eq!(
            session.messages.len(),
            1,
            "transcript collapses to 1 handoff marker"
        );
        assert_eq!(session.agent.name, "act", "switches to act");
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // The plan is preserved, not replaced by the blank sentinel.
        assert_eq!(session.handoff_plan.as_deref(), Some("plan text"));
        assert!(
            session.messages[0].text().contains("plan text"),
            "marker carries the preserved plan"
        );

        let has_switch = evs
            .iter()
            .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act"));
        let has_reset = evs
            .iter()
            .any(|e| matches!(e, SessionEvent::TranscriptReset(_)));
        let has_handoff = evs
            .iter()
            .any(|e| matches!(e, SessionEvent::PlanHandoff(p) if p == "plan text"));
        assert!(has_switch, "AgentSwitch(act) emitted");
        assert!(has_reset, "TranscriptReset emitted");
        assert!(has_handoff, "PlanHandoff emitted carrying the plan");
        // AgentSwitch must come before TranscriptReset.
        let switch_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::AgentSwitch(_)));
        let reset_idx = evs
            .iter()
            .position(|e| matches!(e, SessionEvent::TranscriptReset(_)));
        assert!(switch_idx < reset_idx, "AgentSwitch before TranscriptReset");
    }

    #[tokio::test]
    async fn apply_clear_context_no_plan_falls_back_to_fresh_start() {
        // No assistant plan text exists -> ClearContext falls back to the blank
        // fresh-start sentinel path (no plan to hand off).
        let mut session = make_session(None);
        session.messages.push(Message::user("u1", "hello"));
        session.messages.push(Message::user("u2", "still no plan"));

        let evs = collect_events(&mut session, ControlCmd::ClearContext);

        assert_eq!(
            session.messages.len(),
            1,
            "transcript collapses to 1 fresh-start marker"
        );
        assert_eq!(session.agent.name, "act", "switches to act");
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // No plan -> blank sentinel stored so resume reconstructs fresh-start.
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some(CLEAR_CONTEXT_SENTINEL),
        );
        assert!(
            session.messages[0].text().contains("Context cleared"),
            "marker is the blank fresh-start"
        );

        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act")),
            "AgentSwitch(act) emitted"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
        assert!(
            !evs.iter()
                .any(|e| matches!(e, SessionEvent::PlanHandoff(_))),
            "no PlanHandoff when there is no plan"
        );
    }

    #[tokio::test]
    async fn apply_clear_context_clears_skill_in_store() {
        // Regression: /act_clear_context cleared the in-memory skill but left
        // the store's `skill` column populated, so resume() reloaded a stale
        // skill. Both layers must now be empty after the clear.
        let store =
            Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn opencoder_store::Store>;
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "sess-ctrl".into(),
                agent: Some("plan".into()),
                skill: Some("reviewer".into()),
                created_at: 0,
                updated_at: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        let mut session = make_session(Some(store.clone()));
        session.agent = resolve_agent("plan").unwrap();
        session.messages.push(Message::user("u1", "hello"));
        let mut a = Message::assistant("a1");
        a.blocks.push(ContentBlock::text("plan text"));
        session.messages.push(a);
        // A skill is active both in the store and in memory.
        session.set_skill(Some("reviewer".into()));
        assert_eq!(session.skill_prompt_cloned().as_deref(), Some("reviewer"));

        let _ = collect_events(&mut session, ControlCmd::ClearContext);

        // In-memory skill cleared.
        assert_eq!(
            session.skill_prompt_cloned(),
            None,
            "in-memory skill must be cleared"
        );
        // Persisted skill cleared -- the exact regression this guards.
        let persisted = store.get_session("sess-ctrl").await.unwrap().unwrap();
        assert_eq!(
            persisted.skill, None,
            "store skill must be NULL after clear-context (resume must not reload it)"
        );
    }

    #[tokio::test]
    async fn apply_clear_context_with_no_skill_is_harmless() {
        // No skill was ever set: clearing must be a no-op, never panic/error.
        let store =
            Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn opencoder_store::Store>;
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "sess-ctrl".into(),
                agent: Some("plan".into()),
                created_at: 0,
                updated_at: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        let mut session = make_session(Some(store.clone()));
        session.agent = resolve_agent("plan").unwrap();
        session.messages.push(Message::user("u1", "hello"));

        let _ = collect_events(&mut session, ControlCmd::ClearContext);

        assert_eq!(session.skill_prompt_cloned(), None);
        let persisted = store.get_session("sess-ctrl").await.unwrap().unwrap();
        assert_eq!(persisted.skill, None, "skill stays None");
    }

    #[test]
    fn apply_switch_noop_for_unknown_agent() {
        let mut session = make_session(None);
        let evs = collect_events(&mut session, ControlCmd::SwitchAgent("nonexistent".into()));
        assert_eq!(session.agent.name, "act", "unchanged");
        assert!(evs.is_empty(), "no events for unknown agent");
    }
}
