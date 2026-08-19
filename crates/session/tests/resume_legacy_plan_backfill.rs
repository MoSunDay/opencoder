//! Regression: legacy sessions (created before the plan-phase columns existed,
//! deployed `860831d`) persist `plan_input_count=0` and `plan_snapshot=NULL`
//! even when the plan agent produced a real plan. On resume that left the
//! plan→act handoff unarmed — Shift+Tab degraded to a plain switch and
//! `/act_clear_context` took the blank-fresh-start path (wiped all context).
//!
//! The backfill must be PHASE-BOUNDED (the `ecce7b0` anti-fabrication
//! guarantee): only assistant text tagged `agent == "plan"` may be recovered
//! as the plan. A session whose plan phase produced NO output must keep the
//! unarmed state — never wrap an earlier act-mode answer as a "plan".

use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message};
use opencoder_llm::MockChatClient;
use opencoder_session::resume;
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn cfg() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn assistant(id: &str, agent: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m.agent = Some(agent.into());
    m
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// A legacy plan session: `agent="plan"`, plan-phase columns empty (0/NULL),
/// but the transcript holds a real plan-mode requirement + plan answer.
async fn resume_legacy(id: &str, store: Arc<dyn Store>) -> opencoder_session::SessionState {
    resume(
        store,
        id,
        cfg(),
        Arc::new(MockChatClient::new()),
        tempfile::tempdir().unwrap().path().to_path_buf(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn legacy_plan_session_backfills_snapshot_and_counter() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "legacy-plan".into(),
            agent: Some("plan".into()),
            plan_snapshot: None,
            plan_input_count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    // Legacy transcript: an act-mode exchange followed by the plan phase.
    store
        .append_messages(
            "legacy-plan",
            &[
                Message::user("u1", "do task X"),
                assistant("a1", "act", "task done"),
                Message::user("u2", "plan feature Y"),
                assistant("a2", "plan", "## Plan\n1. do X\n2. do Y"),
            ],
        )
        .await
        .unwrap();

    let s = resume_legacy("legacy-plan", store).await;
    assert_eq!(
        s.plan_input_count, 1,
        "legacy plan requirement must backfill the counter to 1"
    );
    assert_eq!(
        s.plan_snapshot.as_deref(),
        Some("## Plan\n1. do X\n2. do Y"),
        "legacy plan snapshot must be recovered from the plan-agent answer"
    );
}

#[tokio::test]
async fn legacy_failed_plan_phase_never_backfills_act_answer() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "legacy-failed".into(),
            agent: Some("plan".into()),
            plan_snapshot: None,
            plan_input_count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    // The ecce7b0 fabrication scenario: act answer first, then a plan
    // requirement whose turn died BEFORE any output — no plan-agent text.
    store
        .append_messages(
            "legacy-failed",
            &[
                Message::user("u1", "do task X"),
                assistant("a1", "act", "task done"),
                Message::user("u2", "plan feature Y"),
            ],
        )
        .await
        .unwrap();

    let s = resume_legacy("legacy-failed", store).await;
    assert_eq!(
        s.plan_input_count, 0,
        "a plan phase with no output must not arm the handoff"
    );
    assert_eq!(
        s.plan_snapshot, None,
        "the earlier act answer must NEVER be wrapped as a plan"
    );
}

#[tokio::test]
async fn legacy_plan_phase_with_act_answer_tail_stays_unarmed() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "legacy-act-tail".into(),
            agent: Some("plan".into()),
            plan_snapshot: None,
            plan_input_count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    // Session is in plan mode but the LAST assistant text was produced by the
    // act agent (plan phase produced nothing after it) — must stay unarmed.
    store
        .append_messages(
            "legacy-act-tail",
            &[
                Message::user("u1", "do task X"),
                assistant("a1", "act", "task done"),
            ],
        )
        .await
        .unwrap();

    let s = resume_legacy("legacy-act-tail", store).await;
    assert_eq!(s.plan_input_count, 0);
    assert_eq!(s.plan_snapshot, None);
}

#[tokio::test]
async fn legacy_act_session_never_backfills() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "legacy-act".into(),
            agent: Some("act".into()),
            plan_snapshot: None,
            plan_input_count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            "legacy-act",
            &[
                Message::user("u1", "do task X"),
                assistant("a1", "act", "task done"),
            ],
        )
        .await
        .unwrap();

    let s = resume_legacy("legacy-act", store).await;
    assert_eq!(s.plan_input_count, 0, "act sessions never arm the handoff");
    assert_eq!(s.plan_snapshot, None);
}

#[tokio::test]
async fn persisted_plan_state_is_never_overwritten() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "persisted".into(),
            agent: Some("plan".into()),
            plan_snapshot: Some("## Plan\nreal".into()),
            plan_input_count: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            "persisted",
            &[
                Message::user("u1", "plan feature Y"),
                assistant("a1", "plan", "## Plan\nreal"),
            ],
        )
        .await
        .unwrap();

    let s = resume_legacy("persisted", store).await;
    assert_eq!(
        s.plan_input_count, 2,
        "persisted counter must be preserved, not re-derived"
    );
    assert_eq!(
        s.plan_snapshot.as_deref(),
        Some("## Plan\nreal"),
        "persisted snapshot must be preserved, not re-derived"
    );
}

/// ts-origin legacy session: the session row's agent column is NULL by design,
/// yet a plan-agent answer in the transcript must still backfill the phase
/// state (the NULL agent must be let through the gate).
#[tokio::test]
async fn legacy_ts_origin_null_agent_session_backfills() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "legacy-ts".into(),
            agent: None,
            plan_snapshot: None,
            plan_input_count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            "legacy-ts",
            &[
                Message::user("u1", "plan feature Y"),
                assistant("a1", "plan", "## Plan\n1. do X\n2. do Y"),
            ],
        )
        .await
        .unwrap();

    let s = resume_legacy("legacy-ts", store).await;
    assert_eq!(
        s.plan_input_count, 1,
        "NULL-agent ts-origin rows must backfill the counter"
    );
    assert_eq!(
        s.plan_snapshot.as_deref(),
        Some("## Plan\n1. do X\n2. do Y"),
        "NULL-agent ts-origin rows must backfill the snapshot"
    );
}
