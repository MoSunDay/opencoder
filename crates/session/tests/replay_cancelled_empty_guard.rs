//! Regression for Bug 6: when the replay loop breaks immediately (cancelled
//! token) and `backfill` stays empty, the empty guard must skip recording a
//! synthetic Tool message — otherwise the transcript is corrupted with a stray
//! empty tool_result that providers reject.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::resume::replay_cancelled_tasks;
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn replay_cancelled_tasks_no_empty_tool_message_when_backfill_empty() {
    // When the replay loop breaks immediately (cancelled token), `backfill`
    // stays empty. The empty guard must skip recording a synthetic Tool
    // message — otherwise the transcript is corrupted with a stray empty
    // tool_result that providers reject.
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "s1".into(),
            agent: Some("act".into()),
            model: Some("m".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    // child_session_id is also a FK → create the child session row too.
    store
        .create_session(&SessionMeta {
            id: "c1".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // A cancelled subagent task with NO matching tool_result in the
    // transcript → eligible for replay.
    let task = SubagentTaskRecord {
        task_id: "t1".into(),
        parent_session_id: "s1".into(),
        child_session_id: "c1".into(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "do something".into(),
        result: None,
        status: SubagentStatus::Cancelled,
        ok: None,
        started_at: 0,
        completed_at: None,
    };
    store.create_subagent_task(&task).await.unwrap();

    let agent = resolve_agent("act").unwrap();
    let config = Config {
        model: "m".into(),
        ..Config::default()
    };
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut session = SessionState::new("s1", agent, config, client, PathBuf::from("/tmp"));
    session.store = Some(store.clone());

    // Cancel immediately so the replay loop breaks on the first iteration.
    let token = CancellationToken::new();
    token.cancel();
    session.cancel = Some(token);

    let msg_count_before = session.messages.len();
    replay_cancelled_tasks(&mut session, false).await;
    // No empty Tool message should have been recorded.
    assert_eq!(
        session.messages.len(),
        msg_count_before,
        "no empty Tool message should be recorded when backfill is empty"
    );
    // Belt-and-suspenders: assert no Role::Tool message exists at all.
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.blocks.is_empty()),
        "transcript must not contain an empty Tool message"
    );
}
