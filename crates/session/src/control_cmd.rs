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

/// Parse a user prompt into a control command. Returns `None` for anything that
/// is not an exact (whitespace-trimmed) match for `/act`, `/plan`, or
/// `/act_clear_context`.
pub fn parse(prompt: &str) -> Option<ControlCmd> {
    match prompt.trim() {
        "/act" => Some(ControlCmd::SwitchAgent("act".into())),
        "/plan" => Some(ControlCmd::SwitchAgent("plan".into())),
        "/act_clear_context" => Some(ControlCmd::ClearContext),
        _ => None,
    }
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
                persist_agent(session, name).await;
                on_event(SessionEvent::AgentSwitch(name.clone()));
            }
        }
        ControlCmd::ClearContext => {
            // Total store messages that predate the clear (the history to trim
            // on resume). Accounts for any in-memory-only compaction summary.
            let store_msg_count = session.store_message_count();

            // Preserve recent images so they travel into the fresh transcript.
            let preserved_images = crate::compaction::collect_head_images(&session.messages);
            let mut marker = fresh_start_message();
            for url in &preserved_images {
                marker
                    .blocks
                    .push(ContentBlock::Image { url: url.clone(), detail: None });
            }
            session.messages = vec![marker];
            // Record the boundary so resume reconstructs the fresh marker, not
            // the full cleared history.
            session.after_handoff(store_msg_count as i64, CLEAR_CONTEXT_SENTINEL.to_string());

            // Clear context always switches to act.
            if let Some(a) = resolve_agent("act") {
                session.agent = a;
            }
            session.set_skill(None);

            persist_clear(session).await;
            on_event(SessionEvent::AgentSwitch("act".into()));
            on_event(SessionEvent::TranscriptReset(session.messages.clone()));
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

/// Persist an agent switch to the store. This also closes the latent
/// resume-persistence gap where a mode switch via `UiCmd::SwitchAgent` was not
/// always durably recorded.
async fn persist_agent(session: &SessionState, agent: &str) {
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
    fn parse_exact_matches() {
        assert_eq!(parse("/act"), Some(ControlCmd::SwitchAgent("act".into())));
        assert_eq!(parse("/plan"), Some(ControlCmd::SwitchAgent("plan".into())));
        assert_eq!(parse("/act_clear_context"), Some(ControlCmd::ClearContext));
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(parse("  /plan  "), Some(ControlCmd::SwitchAgent("plan".into())));
        assert_eq!(parse("\t/act\n"), Some(ControlCmd::SwitchAgent("act".into())));
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

    fn collect_events(
        session: &mut SessionState,
        cmd: ControlCmd,
    ) -> Vec<SessionEvent> {
        let mut evs = Vec::new();
        let mut on_event = |ev: SessionEvent| evs.push(ev);
        let _ = futures::executor::block_on(apply(session, &cmd, &mut on_event));
        evs
    }

    #[tokio::test]
    async fn apply_switch_agent_changes_agent_and_emits() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn opencoder_store::Store>;
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
        assert!(evs.iter().any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "plan")));

        // Persisted to the store.
        let meta = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(meta.agent.as_deref(), Some("plan"));
    }

    #[tokio::test]
    async fn apply_clear_context_collapses_and_emits() {
        let store = Arc::new(LibsqlStore::open_memory().await.unwrap()) as Arc<dyn opencoder_store::Store>;
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

        assert_eq!(session.messages.len(), 1, "transcript collapses to 1 marker");
        assert_eq!(session.agent.name, "act", "switches to act");
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some(CLEAR_CONTEXT_SENTINEL),
        );

        let has_switch = evs.iter().any(|e| matches!(e, SessionEvent::AgentSwitch(a) if a == "act"));
        let has_reset = evs.iter().any(|e| matches!(e, SessionEvent::TranscriptReset(_)));
        assert!(has_switch, "AgentSwitch(act) emitted");
        assert!(has_reset, "TranscriptReset emitted");
        // AgentSwitch must come before TranscriptReset.
        let switch_idx = evs.iter().position(|e| matches!(e, SessionEvent::AgentSwitch(_)));
        let reset_idx = evs.iter().position(|e| matches!(e, SessionEvent::TranscriptReset(_)));
        assert!(switch_idx < reset_idx, "AgentSwitch before TranscriptReset");
    }

    #[test]
    fn apply_switch_noop_for_unknown_agent() {
        let mut session = make_session(None);
        let evs = collect_events(&mut session, ControlCmd::SwitchAgent("nonexistent".into()));
        assert_eq!(session.agent.name, "act", "unchanged");
        assert!(evs.is_empty(), "no events for unknown agent");
    }
}
