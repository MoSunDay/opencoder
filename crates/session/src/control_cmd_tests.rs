//! Unit tests for [`crate::control_cmd`] (parse + apply), split out of
//! the source file to respect its line budget.

use super::*;
use std::sync::Arc;

fn parse(s: &str) -> Option<ControlCmd> {
    super::parse(s)
}

#[test]
fn parse_bare_commands() {
    assert_eq!(parse("/act"), Some(ControlCmd::SwitchAgent("act".into())));
    assert_eq!(parse("/plan"), Some(ControlCmd::SwitchAgent("plan".into())));
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
fn consumed_echo_tails_compound_suppresses_bare_keeps_plain() {
    // Plain text echoes verbatim — it is exactly what enters context.
    assert_eq!(
        consumed_echo_text("review the code"),
        Some("review the code".to_string())
    );
    // Compound echoes only the tail: the command token is applied inline
    // and never recorded, so it must never be echoed either.
    assert_eq!(
        consumed_echo_text("/plan review"),
        Some("review".to_string())
    );
    assert_eq!(
        consumed_echo_text("/act_clear_context finish the summary"),
        Some("finish the summary".to_string())
    );
    // Bare command: applied inline, nothing recorded, nothing echoed.
    assert_eq!(consumed_echo_text("/plan"), None);
    assert_eq!(consumed_echo_text("/act   "), None);
    assert_eq!(consumed_echo_text("/clear_context"), None);
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
    // Plan clear converges to act in the same patch as the boundary.
    let meta = store.get_session("sess-ctrl").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
}

#[tokio::test]
async fn apply_clear_context_clears_active_skill_names_too() {
    // Regression: /clear_context wiped only `skill_prompt` and left
    // `active_skill_names` stale - the names set is the other latent-tool
    // lock, so ssh_pty/question stayed unlocked (and the `[active skill]`
    // tail reminder stayed armed) across the clear boundary. Both locks
    // must go together (clear_skill_state seam).
    let mut session = make_session(None);
    session.messages.push(Message::user("u1", "hello"));
    let mut a = Message::assistant("a1");
    a.blocks.push(ContentBlock::text("reply"));
    session.messages.push(a);
    session.set_skill(Some("> Source: /skills/task-plan/SKILL.md\n\nPLAN".into()));
    session.set_active_skill_names(["task-plan".into()].into_iter().collect());
    assert!(!session.active_skill_names_cloned().is_empty());

    let _ = collect_events(&mut session, ControlCmd::ClearContext);

    assert_eq!(session.skill_prompt_cloned(), None, "skill body cleared");
    assert!(
        session.active_skill_names_cloned().is_empty(),
        "active_skill_names must be cleared with the body"
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

#[tokio::test]
async fn apply_clear_context_on_plan_hands_off_to_act() {
    // The canonical clear is the plan execution boundary: preserve the
    // plan under an explicit execution directive, then converge to act.
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
    let working_dir = std::env::temp_dir().join("opencoder-control-cmd-tests");
    let mut session = SessionState::new(
        "sess-ctrl",
        resolve_agent("plan").unwrap(),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        working_dir,
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages.push(Message::user("u1", "do task X"));
    let mut reply = Message::assistant("a1");
    reply.blocks.push(ContentBlock::text("task done"));
    session.messages.push(reply);

    let evs = collect_events(&mut session, ControlCmd::ClearContext);

    assert_eq!(session.agent.name, "act", "clear converges to act");
    assert_eq!(evs.len(), 2, "reset then switch, got: {evs:?}");
    assert!(
        matches!(evs[0], SessionEvent::TranscriptReset(_)),
        "first event resets the transcript: {evs:?}"
    );
    assert!(
        matches!(&evs[1], SessionEvent::AgentSwitch(name) if name == "act"),
        "second event switches to act: {evs:?}"
    );
    assert!(
        session.messages[0].text().contains("Execute it now"),
        "preserved plan must be an execution directive"
    );
    assert_eq!(
        session.handoff_plan.as_deref(),
        Some("task done"),
        "handoff metadata stores display plan without directive framing"
    );
    let meta = store.get_session("sess-ctrl").await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
    assert_eq!(meta.handoff_plan.as_deref(), Some("task done"));
}

#[test]
fn apply_switch_noop_for_unknown_agent() {
    let mut session = make_session(None);
    let evs = collect_events(&mut session, ControlCmd::SwitchAgent("nonexistent".into()));
    assert_eq!(session.agent.name, "act", "unchanged");
    // The silent no-op is fixed: a typo'd `/agent <name>` must name the
    // agent in an Error event so the user knows the switch failed.
    assert_eq!(evs.len(), 1, "exactly one Error event, got: {evs:?}");
    assert!(
        matches!(&evs[0], SessionEvent::Error(e) if e.contains("nonexistent")),
        "error names the agent: {evs:?}"
    );
}
