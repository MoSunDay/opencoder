//! Regression: the plan→act handoff must be phase-bounded.
//!
//! Bug (fixed): `handoff` extracted "the last non-empty assistant text" from
//! the WHOLE transcript with no phase boundary. When a plan-phase requirement
//! was submitted (`plan_input_count` increments BEFORE the LLM call, in
//! `maybe_tag_plan_prompt`) but the turn then failed or was cancelled before
//! the model produced any output, the extraction picked the LAST ACT-PHASE
//! answer instead, wrapped it in the plan→act directive, collapsed the
//! transcript and persisted an irreversible `handoff_seq` boundary — the user
//! saw "Shift+Tab wiped all context and kept no plan".
//!
//! Contracts (new semantics — `handoff` reads ONLY the phase-bounded
//! `plan_snapshot`, captured by `SessionState::record` while the plan agent
//! actually answers):
//! - a plan phase with submitted input but NO plan output hands off NOTHING:
//!   `handoff` returns `None`, the transcript stays intact, no `handoff_seq`;
//! - a failed plan turn records no snapshot (record of the error marker /
//!   cancelled turn does not fabricate one);
//! - the happy path (plan agent answers) still hands the real plan forward.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::{plan_handoff, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

#[tokio::test]
async fn failed_plan_phase_hands_off_nothing_even_with_act_history() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "no-fabricate".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock: Arc<MockChatClient> = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "no-fabricate",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    // Earlier ACT phase: a normal question/answer pair. Pre-fix, this answer
    // is exactly what `final_plan_text` would extract and wrap as a "plan".
    session
        .record(Message::user("u1", "implement feature X"))
        .await;
    let mut answer = Message::assistant("a1");
    answer.blocks.push(opencoder_core::ContentBlock::text(
        "task complete — implemented X",
    ));
    session.record(answer).await;

    // Plan phase: switch to plan, submit a requirement (counter increments
    // pre-LLM), but the turn dies before any assistant output — simulate by
    // recording the user prompt exactly like `maybe_tag_plan_prompt` + record
    // would, with NO assistant message ever recorded.
    session.agent = resolve_agent("plan").unwrap();
    session.maybe_tag_plan_prompt(&mut "plan feature Y".to_string());
    session.record(Message::user("u2", "plan feature Y")).await;
    session.persist_plan_phase().await;
    assert!(session.plan_input_count > 0, "requirement counted pre-LLM");
    assert_eq!(session.plan_snapshot, None, "no plan output this phase");

    // Handoff must refuse: no fold, no boundary, transcript intact.
    assert!(
        plan_handoff::handoff(&mut session, "").is_none(),
        "a phase with no plan output must not hand off"
    );
    assert_eq!(session.messages.len(), 3, "transcript must stay intact");
    assert!(session.handoff_seq.is_none(), "no resume boundary written");
    let meta = store.get_session("no-fabricate").await.unwrap().unwrap();
    assert!(
        meta.handoff_seq.is_none(),
        "store keeps no handoff boundary"
    );
}

#[tokio::test]
async fn recorded_plan_output_is_handed_off_from_the_snapshot() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "real-plan".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock: Arc<MockChatClient> = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "real-plan",
        resolve_agent("plan").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();

    session.record(Message::user("u1", "plan feature Y")).await;
    let mut plan = Message::assistant("a1");
    plan.blocks
        .push(opencoder_core::ContentBlock::text("## Plan\n1. do Y"));
    // The normal runner path: recording the plan assistant turn captures the
    // phase snapshot.
    session.record(plan).await;
    assert_eq!(session.plan_snapshot.as_deref(), Some("## Plan\n1. do Y"));

    // Switch to act (plain switch, snapshot survives) and hand off.
    session.agent = resolve_agent("act").unwrap();
    let display = plan_handoff::handoff(&mut session, "and start now")
        .expect("a recorded plan must hand off");
    assert!(display.contains("## Plan\n1. do Y"));
    assert!(display.contains("and start now"));
    assert_eq!(
        session.messages.len(),
        1,
        "transcript collapsed to the plan"
    );
    assert!(session.handoff_seq.is_some(), "resume boundary written");
    assert_eq!(session.plan_snapshot, None, "snapshot consumed by handoff");

    // The durable mirror is un-armed once the caller persists the boundary
    // (mirrors the TUI worker's update_session patch).
    store
        .update_session(
            "real-plan",
            &opencoder_store::SessionPatch {
                handoff_seq: session.handoff_seq,
                handoff_plan: session.handoff_plan.clone(),
                clear_plan_snapshot: true,
                plan_input_count: Some(session.plan_input_count as i64),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let meta = store.get_session("real-plan").await.unwrap().unwrap();
    assert_eq!(meta.plan_snapshot, None);
    assert_eq!(meta.plan_input_count, 0);
}
