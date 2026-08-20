//! Compaction ⇄ plan-handoff interaction: a long plan session that triggers
//! auto-compaction folds the plan (an assistant message) into the user-role
//! summary head, after which `final_plan_text` can no longer find it — the
//! root cause of "Shift+Tab after a long plan run silently does a plain mode
//! swap and act runs with the full planning transcript".
//!
//! Contracts:
//! - compaction in plan mode captures `plan_snapshot` BEFORE replacing the
//!   transcript and persists it (plus the plan-input counter) to the store;
//! - a later `handoff` falls back to that snapshot, resets the transcript,
//!   and consumes the snapshot;
//! - a second compaction with no assistant text keeps the existing snapshot;
//! - compaction in act mode never captures a snapshot;
//! - `resume` restores both fields so the TUI re-arms Shift+Tab.

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role, ToolArc};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::compaction::compact;
use opencoder_session::resume;
use opencoder_session::{plan_handoff, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

const PLAN_TEXT: &str = "## Plan\n1. implement the snapshot fallback\n2. add tests";

fn cfg() -> Config {
    Config {
        model: "m/g".into(),
        compaction: opencoder_core::CompactionConfig {
            tail_turns: 1,
            ..Config::default().compaction
        },
        ..Config::default()
    }
}

fn user(id: &str, text: &str) -> Message {
    Message::user(id, text)
}

fn assistant(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn summary_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("summary of the planning talk".into()),
        LlmEvent::Completed {
            text: "summary of the planning talk".into(),
            tool_calls: Vec::<CompletedToolCall>::new(),
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 1,
                total_tokens: 2,
                ..Usage::default()
            }),
        },
    ]))
}

fn plan_session(store: Arc<dyn Store>, agent: &str) -> SessionState {
    SessionState::new(
        "plan-snap",
        resolve_agent(agent).unwrap(),
        cfg(),
        summary_client(),
        std::env::temp_dir(),
    )
    .with_store(store)
}

async fn seed_plan_store(store: &Arc<dyn Store>) {
    store
        .create_session(&SessionMeta {
            id: "plan-snap".into(),
            title: None,
            agent: Some("plan".into()),
            model: Some("m/g".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
    // u1 → plan answer → trailing unanswered user question. tail_turns=1
    // keeps only the trailing turn, so the plan (assistant) lands in the
    // compacted head exactly as in the bug report.
    let msgs = vec![
        user("u1", "plan the feature"),
        assistant("a1", PLAN_TEXT),
        user("u2", "one more clarifying question"),
    ];
    store.append_messages("plan-snap", &msgs).await.unwrap();
}

#[tokio::test]
async fn compaction_snapshots_plan_and_handoff_recovers() {
    let store = mem_store().await;
    seed_plan_store(&store).await;
    let mut session = plan_session(store.clone(), "plan");
    session.messages = vec![
        user("u1", "plan the feature"),
        assistant("a1", PLAN_TEXT),
        user("u2", "one more clarifying question"),
    ];
    session.plan_input_count = 2; // two plan prompts submitted this phase

    let registry: HashMap<String, ToolArc> = opencoder_session::tools::registry();
    let summary = compact(&mut session, &registry, &mut |_| {})
        .await
        .expect("compaction must succeed")
        .expect("compaction must summarize");

    assert!(!summary.is_empty());
    assert_eq!(
        session.plan_snapshot.as_deref(),
        Some(PLAN_TEXT),
        "compaction must capture the plan before folding the head"
    );
    assert!(
        plan_handoff::final_plan_text(&session.messages).is_none(),
        "post-compaction tail has no assistant text (the folded-plan shape)"
    );
    assert_eq!(session.messages[0].role, Role::User);

    // The plan phase is durably recorded for resume.
    let meta = store.get_session("plan-snap").await.unwrap().unwrap();
    assert_eq!(meta.plan_snapshot.as_deref(), Some(PLAN_TEXT));
    assert_eq!(meta.plan_input_count, 2);

    // Handoff recovers via the snapshot: transcript reset + plan carried.
    let display = plan_handoff::handoff(&mut session, "").expect("handoff must find the plan");
    assert_eq!(display, PLAN_TEXT);
    assert_eq!(
        session.messages.len(),
        1,
        "transcript collapsed to the plan"
    );
    assert!(session.messages[0].synthetic);
    assert_eq!(session.plan_snapshot, None, "snapshot consumed");

    // Persist the boundary exactly like the TUI worker path, then resume.
    store
        .update_session(
            "plan-snap",
            &SessionPatch {
                agent: Some("act".into()),
                handoff_seq: session.handoff_seq,
                handoff_plan: session.handoff_plan.clone(),
                clear_summary: true,
                clear_plan_snapshot: true,
                plan_input_count: Some(session.plan_input_count as i64),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let resumed = resume(
        store.clone(),
        "plan-snap",
        cfg(),
        summary_client(),
        dir.path().to_path_buf(),
    )
    .await
    .expect("resume must succeed");

    assert_eq!(resumed.messages.len(), 1, "resume rebuilds the handoff msg");
    assert!(resumed.messages[0].text().contains(PLAN_TEXT));
    assert_eq!(resumed.plan_input_count, 0, "act phase starts un-armed");
    assert_eq!(resumed.plan_snapshot, None);
}

#[tokio::test]
async fn second_compaction_without_plan_keeps_existing_snapshot() {
    let store = mem_store().await;
    seed_plan_store(&store).await;
    let mut session = plan_session(store.clone(), "plan");
    session.messages = vec![
        user("u1", "plan the feature"),
        assistant("a1", PLAN_TEXT),
        user("u2", "one more clarifying question"),
    ];

    let registry = opencoder_session::tools::registry();
    compact(&mut session, &registry, &mut |_| {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.plan_snapshot.as_deref(), Some(PLAN_TEXT));

    // More user-only chatter, then compact again: `final_plan_text` misses,
    // so the snapshot must survive untouched (no overwrite, no drop).
    for (i, text) in ["u3: question", "u4: another question"].iter().enumerate() {
        let m = user(&format!("extra-{i}"), text);
        store.append_message("plan-snap", &m).await.unwrap();
        session.messages.push(m);
    }
    compact(&mut session, &registry, &mut |_| {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        session.plan_snapshot.as_deref(),
        Some(PLAN_TEXT),
        "a miss must not overwrite the captured snapshot"
    );
}

#[tokio::test]
async fn act_mode_compaction_never_captures_snapshot() {
    let store = mem_store().await;
    seed_plan_store(&store).await;
    let mut session = plan_session(store.clone(), "act");
    session.messages = vec![
        user("u1", "plan the feature"),
        assistant("a1", PLAN_TEXT),
        user("u2", "one more clarifying question"),
    ];

    let registry = opencoder_session::tools::registry();
    compact(&mut session, &registry, &mut |_| {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        session.plan_snapshot, None,
        "act-mode compaction must not fabricate plan provenance"
    );
}

#[tokio::test]
async fn resume_restores_plan_phase_arming() {
    let store = mem_store().await;
    seed_plan_store(&store).await;
    store
        .update_session(
            "plan-snap",
            &SessionPatch {
                plan_snapshot: Some(PLAN_TEXT.into()),
                plan_input_count: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let resumed = resume(
        store,
        "plan-snap",
        cfg(),
        summary_client(),
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert_eq!(resumed.agent.name, "plan");
    assert_eq!(
        resumed.plan_input_count, 3,
        "arming counter must survive resume (TUI Shift+Tab re-arm)"
    );
    assert_eq!(resumed.plan_snapshot.as_deref(), Some(PLAN_TEXT));
}
