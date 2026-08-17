//! Shared in-crate test fixtures for the runner's steer/drain unit tests.
//!
//! Extracted from `steer.rs`'s test module so the steer and drain suites
//! share the same in-memory store/session builders without duplication,
//! keeping each file within the per-file size gate.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};
use tokio_util::sync::CancellationToken;

use crate::{SessionState, SharedCancel};

pub(super) fn mock_client() -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "ok".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Open an in-memory store, seed the session row, and build a SessionState
/// wired to it. Shared by all queue/drain test setups. The caller attaches
/// a turn_cancel token and seeds inputs as needed.
pub(super) async fn make_session(id: &str) -> (SessionState, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            title: Some("test".into()),
            agent: Some("act".into()),
            model: Some("m/g".into()),
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
        })
        .await
        .unwrap();
    let agent = resolve_agent("act").unwrap();
    let config = Config {
        model: "m/g".into(),
        ..Default::default()
    };
    let session = SessionState::new(id, agent, config, mock_client(), std::env::temp_dir())
        .with_store(store.clone());
    (session, store)
}

/// Build a SessionState wired to an in-memory store that already has one
/// pending Steer input and one pending Queue input.
pub(super) async fn session_with_pending() -> (SessionState, Arc<dyn Store>, SharedCancel) {
    let (mut session, store) = make_session("cancel-guard-test").await;

    // Admit one steer and one queue input.
    let steer_input = SessionInput {
        seq: None,
        id: "steer-1".into(),
        session_id: "cancel-guard-test".into(),
        delivery: Delivery::Steer,
        prompt: "interrupt!".into(),
        images: vec![],
        admitted_seq: 0,
        promoted_seq: None,
        display_text: None,
    };
    store.admit_input(&steer_input).await.unwrap();

    let queue_input = SessionInput {
        seq: None,
        id: "queue-1".into(),
        session_id: "cancel-guard-test".into(),
        delivery: Delivery::Queue,
        prompt: "queued".into(),
        images: vec![],
        admitted_seq: 0,
        promoted_seq: None,
        display_text: None,
    };
    store.admit_input(&queue_input).await.unwrap();

    let token: SharedCancel = Arc::new(std::sync::Mutex::new(CancellationToken::new()));
    session = session.with_turn_cancel(token.clone());

    (session, store, token)
}

pub(super) async fn session_with_queue(
    prompts: &[&str],
) -> (SessionState, Arc<dyn Store>, SharedCancel) {
    let (mut session, store) = make_session("drain-test").await;
    for (i, p) in prompts.iter().enumerate() {
        store
            .admit_input(&SessionInput {
                seq: None,
                id: format!("q-{i}"),
                session_id: "drain-test".into(),
                delivery: Delivery::Queue,
                prompt: (*p).into(),
                images: vec![],
                admitted_seq: 0,
                promoted_seq: None,
                display_text: None,
            })
            .await
            .unwrap();
    }
    let token: SharedCancel = Arc::new(std::sync::Mutex::new(CancellationToken::new()));
    session = session.with_turn_cancel(token.clone());
    (session, store, token)
}
