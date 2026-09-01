//! P0-3 invariant lock: non-parent-agent operations must NEVER insert a new
//! session row.
//!
//! Every front-end mutation of an EXISTING session -- agent switches (`/act`,
//! `/plan`), context clear (`/clear_context`), model switch, skill
//! activation -- must express itself exclusively through `update_session` /
//! message / event writes. A hidden `create_session` on these paths would fork
//! the session (a duplicate row in `list_sessions`), so each test drives the
//! link against the counting spy in `mod spy_store` and asserts:
//!
//! - the `create_session` call count is unchanged across the operation;
//! - the default-filter `list_sessions` parent set (ids) is identical;
//! - a subagent child row never leaks into the default listing.
//!
//! `/model` note: there is no runner control command for model switching; it is
//! a front-end flow (TUI `UiCmd::ReloadConfig` applied at the turn boundary,
//! web `POST /sessions/:id/model`). Its session-crate seam is
//! `SessionState::apply_config_reload_keep_client` plus an `update_session`
//! model patch -- exactly the sequence reproduced (and locked) here.

#[path = "no_session_row_side_effects/spy_store.rs"]
mod spy_store;

use std::sync::Arc;

use opencoder_core::{Config, Message, Role};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::run;
use opencoder_store::{
    SessionFilter, SessionMeta, Store, SubagentStatus, SubagentTaskRecord, TASK_TYPE_SUBAGENT,
};

use spy_store::{
    done_turn, lock_home, mk_queue_input, mk_session, parent_ids, seed, spy_store, HomeGuard,
    SpyStore, SESS,
};

/// The invariant, asserted uniformly per link: no `create_session` fired and
/// the parent session set is identical to the pre-operation snapshot.
async fn assert_no_new_row(
    spy: &Arc<SpyStore>,
    store: &Arc<dyn Store>,
    before_ids: &[String],
    before_creates: usize,
    ctx: &str,
) {
    assert_eq!(
        spy.creates(),
        before_creates,
        "{ctx}: create_session must not fire"
    );
    assert_eq!(parent_ids(store).await, before_ids, "{ctx}: parent set forked");
}

// ---------------------------------------------------------------------------
// a) bare agent switches: /plan and /act (each a real switch, no no-op)
// ---------------------------------------------------------------------------

async fn assert_bare_switch_no_new_row(seed_agent: &str, cmd: &str, expect_agent: &str) {
    let (spy, store) = spy_store().await;
    seed(&store, SESS, seed_agent).await;
    let mut session = mk_session(seed_agent, Arc::new(MockChatClient::new()), store.clone());

    let before_ids = parent_ids(&store).await;
    let before_creates = spy.creates();

    run(&mut session, cmd.into(), |_| {}).await.unwrap();

    assert_eq!(session.agent.name, expect_agent, "{cmd} must apply (non-vacuous)");
    assert_no_new_row(&spy, &store, &before_ids, before_creates, cmd).await;

    // The switch persisted in place (update_session on the same row).
    let meta = store.get_session(SESS).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some(expect_agent), "{cmd}: persisted in place");
}

#[tokio::test]
async fn plan_switch_never_creates_session_row() {
    assert_bare_switch_no_new_row("act", "/plan", "plan").await;
}

#[tokio::test]
async fn act_switch_never_creates_session_row() {
    assert_bare_switch_no_new_row("plan", "/act", "act").await;
}

// ---------------------------------------------------------------------------
// b) /clear_context: update_session-only boundary; history not lost
// ---------------------------------------------------------------------------

/// Mirrors tests/control_cmd.rs::clear_context_no_assistant_text_survives_resume
/// (user-only history -> blank fresh-start sentinel, no LLM turn). The bare
/// clear persists ONLY `update_session`: old message rows stay in place (the
/// resume-time trim never deletes at clear time), so `load_messages` is
/// identical across the boundary.
#[tokio::test]
async fn clear_context_never_creates_row_and_keeps_history() {
    let (spy, store) = spy_store().await;
    seed(&store, SESS, "plan").await;

    let history = vec![
        Message::user("u1", "old question"),
        Message::user("u2", "another question"),
    ];
    store.append_messages(SESS, &history).await.unwrap();

    let mut session = mk_session("plan", Arc::new(MockChatClient::new()), store.clone());
    session.messages = history.clone();

    let before_ids = parent_ids(&store).await;
    let before_creates = spy.creates();

    run(&mut session, "/clear_context".into(), |_| {}).await.unwrap();

    // The operation really applied: transcript collapsed to the marker and
    // the plan session converged to act without creating another row.
    assert_eq!(session.messages.len(), 1, "transcript collapsed to marker");
    assert_eq!(session.agent.name, "act", "clear converges to act");
    assert_no_new_row(&spy, &store, &before_ids, before_creates, "/clear_context").await;

    // History is not lost: identical persisted rows before and after (the
    // marker lives in memory only; resume owns the trim).
    let digest = |msgs: &[Message]| -> Vec<(Role, String)> {
        msgs.iter().map(|m| (m.role, m.text())).collect()
    };
    assert_eq!(
        digest(&store.load_messages(SESS).await.unwrap()),
        digest(&history),
        "bare clear must not delete (nor append) persisted history"
    );
}

// ---------------------------------------------------------------------------
// c) /model switch: front-end seam (apply_config_reload_keep_client +
//    update_session model patch, as TUI worker / web POST /sessions/:id/model)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn model_switch_never_creates_session_row() {
    let (spy, store) = spy_store().await;
    seed(&store, SESS, "act").await;
    let mut session = mk_session("act", Arc::new(MockChatClient::new()), store.clone());

    let before_ids = parent_ids(&store).await;
    let before_creates = spy.creates();

    // The exact session-crate sequence behind `/model xxx`.
    let new_cfg = Config {
        model: "other/mini".into(),
        ..Config::default()
    };
    session.apply_config_reload_keep_client(new_cfg);
    store
        .update_session(
            SESS,
            &opencoder_store::SessionPatch {
                model: Some(session.config.model.clone()),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(session.config.model, "other/mini", "switch must apply (non-vacuous)");
    assert_no_new_row(&spy, &store, &before_ids, before_creates, "/model").await;

    let meta = store.get_session(SESS).await.unwrap().unwrap();
    assert_eq!(meta.model.as_deref(), Some("other/mini"), "model persisted in place");
}

// ---------------------------------------------------------------------------
// d) skill activation ($review inline token -> one-shot run)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn skill_activation_never_creates_session_row() {
    let home = tempfile::tempdir().unwrap();
    let _guard: HomeGuard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let (spy, store) = spy_store().await;
    seed(&store, SESS, "act").await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff")])
            .push_script(vec![done_turn("work reply")]),
    ) as Arc<dyn ChatStream>;
    let mut session = mk_session("act", mock, store.clone());

    store.admit_input(&mk_queue_input("$review do the work")).await.unwrap();

    let before_ids = parent_ids(&store).await;
    let before_creates = spy.creates();

    run(&mut session, "kickoff".into(), |_| {}).await.unwrap();

    // Non-vacuous: the skill body was injected during the run.
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.text().starts_with("[skill loaded] ")),
        "skill was active during the run"
    );
    assert_no_new_row(&spy, &store, &before_ids, before_creates, "$skill").await;
}

// ---------------------------------------------------------------------------
// e) the listing stays parent-only: a subagent child row never leaks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn listing_stays_parent_only_across_control_ops() {
    let (spy, store) = spy_store().await;
    seed(&store, SESS, "act").await;

    // Child row + parent-child link, exactly as runner/subagent.rs seeds them.
    let child_id = "child-sess";
    store
        .create_session(&SessionMeta {
            id: child_id.into(),
            agent: Some("act".into()),
            task_type: Some(TASK_TYPE_SUBAGENT.into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-1".into(),
            parent_session_id: SESS.into(),
            child_session_id: child_id.into(),
            parent_message_id: None,
            agent: "act".into(),
            prompt: "child prompt".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    // Sanity: the child row exists and is visible only when widened.
    let default_ids = parent_ids(&store).await;
    assert_eq!(default_ids, vec![SESS.to_string()], "default list is parent-only");
    let widened: Vec<String> = store
        .list_sessions(&SessionFilter {
            include_subagents: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert!(widened.contains(&child_id.to_string()), "child row exists (non-vacuous)");

    // Drive a real control op on the parent session.
    let mut session = mk_session("act", Arc::new(MockChatClient::new()), store.clone());
    let before_creates = spy.creates();
    run(&mut session, "/plan".into(), |_| {}).await.unwrap();

    assert_no_new_row(&spy, &store, &default_ids, before_creates, "/plan").await;

    let child_ids: Vec<String> = store
        .list_subagent_tasks(SESS)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.child_session_id)
        .collect();
    assert!(
        default_ids.iter().all(|id| !child_ids.contains(id)),
        "no listed id may reference a subagent_tasks child row"
    );
}
