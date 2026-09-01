//! Queueable control commands: `/act`, `/plan`, `/act_clear_context`
//! (`/clear_context` is the accepted legacy alias).
//!
//! These slash commands switch the runtime agent and/or clear the transcript.
//! Unlike normal prompts, they take effect *immediately* when consumed by the
//! drain loop — they do NOT consume an LLM turn. Public UI admission rejects
//! them while a run is active; the runner parser remains so idle submissions
//! and already-persisted/internal recovery inputs behave deterministically:
//!
//! ```text
//! queue: [/plan] -> [review skill] -> [/act]
//! drain: switch->plan (no turn) . run "review" . switch->act (no turn)
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
/// fresh-start marker (not a directive boundary) after a
/// [`ControlCmd::ClearContext`]. The distinctive ASCII framing guarantees it
/// never collides with real content (no LLM/user output starts with this).
pub(crate) const CLEAR_CONTEXT_SENTINEL: &str = "<<OPENCODER_CLEAR_CONTEXT_MARKER>>";

/// True when a persisted `handoff_plan` is the clear-context sentinel — i.e.
/// the boundary was written by [`ControlCmd::ClearContext`] and preserved
/// nothing. Public so display layers (TUI handoff card, CLI JSON dump) can
/// skip the raw sentinel instead of ever outputting it; the LLM must never
/// see it (resume converts it to [`fresh_start_message`] before rebuilding
/// context).
pub fn is_clear_context_handoff(handoff_plan: &str) -> bool {
    handoff_plan == CLEAR_CONTEXT_SENTINEL
}

/// Marker prefix stored in `handoff_plan` when [`ControlCmd::ClearContext`]
/// preserved the last assistant reply as a continuity seed: the persisted
/// value is this prefix followed by the preserved reply text. Same ASCII
/// framing rationale as [`CLEAR_CONTEXT_SENTINEL`] — it can never collide
/// with real content.
pub(crate) const CLEAR_CONTEXT_SEED_PREFIX: &str = "<<OPENCODER_CLEAR_SEED>>";

/// True when a persisted `handoff_plan` is a clear-context seed boundary —
/// the clear preserved the last assistant reply. Public so display layers
/// (TUI replay, CLI JSON dump) can strip the marker and render the preserved
/// text; the LLM must never see the raw marker (resume converts it back to a
/// [`seed_message`] before rebuilding context).
pub fn is_clear_context_seed(handoff_plan: &str) -> bool {
    handoff_plan.starts_with(CLEAR_CONTEXT_SEED_PREFIX)
}

/// The preserved reply text carried by a clear-context seed boundary (the
/// marker prefix stripped). Only meaningful when [`is_clear_context_seed`]
/// holds.
pub fn clear_seed_text(handoff_plan: &str) -> &str {
    handoff_plan
        .strip_prefix(CLEAR_CONTEXT_SEED_PREFIX)
        .unwrap_or("")
}

/// Marker value persisted in `handoff_plan` for a seed boundary.
fn clear_seed_marker(text: &str) -> String {
    format!("{CLEAR_CONTEXT_SEED_PREFIX}{text}")
}

/// Body of the fresh-start marker message left after a context clear.
const CLEAR_CONTEXT_BODY: &str = "[Context cleared - starting fresh.]";

/// Neutral wrapper for the preserved last say. Deliberately NOT an execution
/// directive ([`crate::handoff`] prefixes the autopilot handoff instead): the
/// preserved text is a plain prior answer ("task done"), not a task, and an
/// execution directive would fabricate a task out of finished work.
const CLEAR_SEED_BODY_PREFIX: &str = "[Context cleared. The previous assistant reply below \
is preserved as continuity context - prior context, not a new instruction.]\n\n";

/// A control command parsed from a slash-command string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlCmd {
    /// Switch the active agent without resetting context.
    SwitchAgent(String),
    /// Clear the transcript, keeping the active agent. Never a full wipe: the
    /// last assistant reply survives as a neutral continuity seed; only a
    /// transcript with no assistant content collapses to the blank
    /// fresh-start marker.
    ClearContext,
}

/// Split a compound prompt into its leading control command and any trailing
/// argument text. Supports `/plan <args>` and `/act <args>` so a single
/// submission like `/plan review` switches agent *and* runs the rest as a
/// prompt in the new agent.
///
/// The clear-context fold is canonicalized as `/act_clear_context` — the
/// `act` prefix kept explicit so the command reads as the act-agent
/// fold-and-restart it is. It supports compound inputs like
/// `/act_clear_context review` where the trailing text runs as a prompt in
/// the fresh context. The legacy spelling `/clear_context` still parses
/// (mapped to the same command) so already-persisted inputs keep behaving
/// deterministically.
///
/// Returns `None` for anything that is not a control command. The rest text is
/// the trimmed remainder after the command token, or `None` when the input was
/// a bare command with nothing following (e.g. `/act`). Inline `$skill`
/// tokens in the rest are preserved verbatim for downstream resolution.
pub fn split_control_prefix(prompt: &str) -> Option<(ControlCmd, Option<String>)> {
    let trimmed = prompt.trim();
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?;
    let cmd = match head {
        "/act" => ControlCmd::SwitchAgent("act".into()),
        "/plan" => ControlCmd::SwitchAgent("plan".into()),
        "/act_clear_context" | "/clear_context" => ControlCmd::ClearContext,
        _ => return None,
    };
    let rest = trimmed
        .strip_prefix(head)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some((cmd, rest))
}

/// PARENT session: queue/steer submissions are admitted and the runner applies
/// them at the next idle/turn boundary, which structurally has no turn in
/// flight. Still the admission guard for subagent steers (subagents have no
/// agent concept) and the TUI's subagent-focus gate.
pub fn is_mode_control(prompt: &str) -> bool {
    split_control_prefix(prompt).is_some()
}

/// Parse a user prompt into a control command. Returns `None` for anything
/// that is not `/act`, `/plan`, `/clear_context` (or the legacy
/// `/act_clear_context`); all accept an optional trailing argument. Compound
/// inputs like `/plan review` are recognized as a control command; use
/// [`split_control_prefix`] to also recover the trailing argument.
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
                // Switching to the agent already in charge is a pure no-op:
                // no persistence write, no AgentSwitch event, no transcript
                // side effects. `/act` on an act session (e.g. the second leg
                // of a `/plan` -> `/act` round trip) must stay silent.
                if session.agent.name == a.name {
                    return Ok(());
                }
                session.agent = a;
                persist_agent(session, name).await?;
                on_event(SessionEvent::AgentSwitch(name.clone()));
            }
        }
        ControlCmd::ClearContext => {
            // Preserve chain (never a full blank wipe): keep the last say —
            // the newest non-empty assistant reply — as a neutral continuity
            // seed. Only a transcript with NO assistant content at all (a
            // brand-new session) degrades to the blank fresh-start sentinel.
            // The seed deliberately travels as prior context in a neutral
            // wrapper, never as an execution instruction.
            //
            // Total store messages that predate the clear (the history to
            // trim on resume). Accounts for any in-memory-only summary.
            let store_msg_count = session.store_message_count();
            let preserved_images = crate::compaction::collect_head_images(&session.messages);
            let (mut marker, boundary) =
                match crate::handoff::last_assistant_text(&session.messages) {
                    Some(last_say) => {
                        let last_say = last_say.trim().to_string();
                        (seed_message(&last_say), clear_seed_marker(&last_say))
                    }
                    None => (fresh_start_message(), CLEAR_CONTEXT_SENTINEL.to_string()),
                };
            for url in &preserved_images {
                marker.blocks.push(ContentBlock::Image {
                    url: url.clone(),
                    detail: None,
                });
            }
            session.messages = vec![marker];
            // Record the boundary (sentinel or seed marker) so resume
            // reconstructs the marker, not the full cleared history.
            session.after_handoff(store_msg_count as i64, boundary);

            session.set_skill(None);
            persist_clear(session).await?;
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

/// Build the synthetic seed message carrying the preserved last assistant
/// reply into the fresh transcript. Exposed so [`crate::resume`] can
/// reconstruct the exact same message after a seed boundary.
pub fn seed_message(text: &str) -> Message {
    let body = format!("{CLEAR_SEED_BODY_PREFIX}{text}");
    let mut msg = Message::user(new_id(), body);
    msg.synthetic = true;
    msg
}

/// Persist an agent switch to the store. Closes the latent
/// resume-persistence gap where a switch via the TUI key handler was not
/// durably recorded: the worker now calls this so `resume()` and the `/task`
/// picker read the switched agent.
pub async fn persist_agent(session: &SessionState, agent: &str) -> Result<()> {
    if let Some(store) = &session.store {
        store
            .update_session(
                &session.id,
                &SessionPatch {
                    agent: Some(agent.into()),
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}

/// Persist the clear-context boundary: handoff metadata + active agent.
async fn persist_clear(session: &SessionState) -> Result<()> {
    if let Some(store) = &session.store {
        store
            .update_session(
                &session.id,
                &SessionPatch {
                    agent: Some(session.agent.name.clone()),
                    handoff_seq: session.handoff_seq,
                    handoff_plan: session.handoff_plan.clone(),
                    clear_summary: true,
                    clear_skill: true,
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn parse(s: &str) -> Option<ControlCmd> {
        super::parse(s)
    }

    #[test]
    fn parse_bare_commands() {
        assert_eq!(parse("/act"), Some(ControlCmd::SwitchAgent("act".into())));
        assert_eq!(
            parse("/plan"),
            Some(ControlCmd::SwitchAgent("plan".into()))
        );
        assert_eq!(parse("/clear_context"), Some(ControlCmd::ClearContext));
        // Legacy spelling still parses so persisted inputs stay deterministic.
        assert_eq!(parse("/act_clear_context"), Some(ControlCmd::ClearContext));
    }

    #[test]
    fn parse_rejects_removed_sandbox_command() {
        // The sandbox-mode interlude is reverted: `/sandbox` is no longer a
        // control command (a queued `/sandbox` degrades to a plain prompt;
        // the CLI rewrites the legacy prefix to `/plan` before it gets here).
        assert_eq!(parse("/sandbox"), None);
        assert_eq!(parse("/sandbox review"), None);
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
        assert_eq!(parse("/clear_ctx"), None);
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
        let (cmd, rest) = split_control_prefix("/clear_context").unwrap();
        assert_eq!(cmd, ControlCmd::ClearContext);
        assert!(rest.is_none());
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
    fn mode_control_predicate_covers_bare_and_compound_commands_only() {
        for prompt in ["/act", "  /plan review  ", "/clear_context continue"] {
            assert!(is_mode_control(prompt), "missed {prompt:?}");
        }
        for prompt in ["", "continue", "/acting", "/compact", "/sandbox"] {
            assert!(!is_mode_control(prompt), "false positive {prompt:?}");
        }
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
        assert_eq!(
            parse("/plan review"),
            Some(ControlCmd::SwitchAgent("plan".into()))
        );
    }

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
    async fn apply_clear_context_preserves_last_say_as_seed() {
        // A prior assistant reply exists -> ClearContext preserves it as a
        // neutral continuity seed rather than wiping to a blank fresh-start.
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
        session.messages.push(Message::user("u1", "hello"));
        let mut a = Message::assistant("a1");
        a.blocks.push(ContentBlock::text("task done"));
        session.messages.push(a);

        let evs = collect_events(&mut session, ControlCmd::ClearContext);

        assert_eq!(
            session.messages.len(),
            1,
            "transcript collapses to 1 seed marker"
        );
        assert!(
            session.agent.name == "act",
            "clear keeps the active agent (no forced switch)"
        );
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // The last say is preserved via the seed marker, not the blank sentinel.
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some(format!("{CLEAR_CONTEXT_SEED_PREFIX}task done").as_str())
        );
        assert!(
            session.messages[0].text().contains("task done"),
            "marker carries the preserved reply as prior context"
        );

        assert!(
            !evs.iter()
                .any(|e| matches!(e, SessionEvent::AgentSwitch(_))),
            "no AgentSwitch: the active agent is kept"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
    }

    #[tokio::test]
    async fn apply_clear_context_no_say_falls_back_to_fresh_start() {
        // No assistant text exists -> ClearContext falls back to the blank
        // fresh-start sentinel path.
        let mut session = make_session(None);
        session.messages.push(Message::user("u1", "hello"));
        session.messages.push(Message::user("u2", "still no reply"));

        let evs = collect_events(&mut session, ControlCmd::ClearContext);

        assert_eq!(
            session.messages.len(),
            1,
            "transcript collapses to 1 fresh-start marker"
        );
        assert!(session.handoff_seq.is_some(), "handoff_seq set");
        // No reply -> blank sentinel stored so resume reconstructs fresh-start.
        assert_eq!(
            session.handoff_plan.as_deref(),
            Some(CLEAR_CONTEXT_SENTINEL)
        );
        assert!(
            session.messages[0].text().contains("Context cleared"),
            "marker is the blank fresh-start"
        );

        assert!(
            evs.iter()
                .any(|e| matches!(e, SessionEvent::TranscriptReset(_))),
            "TranscriptReset emitted"
        );
    }

    #[tokio::test]
    async fn apply_clear_context_clears_skill_in_store() {
        // Regression: /clear_context cleared the in-memory skill but left the
        // store's `skill` column populated, so resume() reloaded a stale
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
        a.blocks.push(ContentBlock::text("reply"));
        session.messages.push(a);
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
        // The active agent survives the clear.
        let meta = store.get_session("sess-ctrl").await.unwrap().unwrap();
        assert_eq!(meta.agent.as_deref(), Some("plan"));
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
