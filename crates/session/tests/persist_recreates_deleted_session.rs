//! Regression for `persist assistant response: insert message in tx:
//! FOREIGN KEY constraint failed` with several tasks sharing one project
//! database.
//!
//! A session row can disappear while its turn is still running: another
//! process's `/task clear all`, web clear, `dialogs_clear` or `session
//! delete` only gates on its own in-process activity, and a storage-level
//! desync can hide the row from the FK probe as well. The next append must
//! not kill the turn — `persist` recreates the row and appends once more.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{Store, TursoStore};

fn make_session(id: &str, store: Arc<dyn Store>) -> SessionState {
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        PathBuf::from("."),
    )
    .with_store(store)
}

fn assistant(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn stored_ids(store: &Arc<dyn Store>, id: &str) -> Vec<String> {
    store
        .load_messages(id)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect()
}

/// `/task clear all` in another window deletes this session mid-turn (the
/// deleting process only knows about its own running tasks).
#[tokio::test]
async fn record_recreates_a_session_cleared_by_another_process() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap()) as Arc<dyn Store>;
    let mut s = make_session("parallel", store.clone());
    s.record(Message::user("u1", "first prompt")).await;
    assert_eq!(stored_ids(&store, "parallel").await, ["u1"]);

    store
        .clear_other_sessions("other-window-session")
        .await
        .unwrap();
    assert!(store.get_session("parallel").await.unwrap().is_none());

    // Before the fix: `insert message in tx: FOREIGN KEY constraint failed`
    // surfaced through `record_checked` and killed the turn; every later
    // persist failed the same way because `session_created` stayed set.
    s.record_checked(assistant("a1", "working")).await.unwrap();

    let meta = store
        .get_session("parallel")
        .await
        .unwrap()
        .expect("session row recreated");
    assert_eq!(meta.title.as_deref(), Some("first prompt"));
    assert_eq!(meta.model.as_deref(), Some("m/g"));
    assert_eq!(
        stored_ids(&store, "parallel").await,
        ["a1"],
        "only the post-deletion tail is stored"
    );
    assert_eq!(s.messages.len(), 2, "in-memory transcript is intact");

    // Later persists go straight to the append again.
    s.record(Message::user("u2", "follow-up")).await;
    assert_eq!(stored_ids(&store, "parallel").await, ["a1", "u2"]);
    assert_eq!(
        store
            .list_sessions(&Default::default())
            .await
            .unwrap()
            .len(),
        1,
        "exactly one session row after the recreation"
    );
}

/// Explicit `session delete` of a running session takes the same path.
#[tokio::test]
async fn record_recreates_an_explicitly_deleted_session() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap()) as Arc<dyn Store>;
    let mut s = make_session("deleted", store.clone());
    s.record(Message::user("u1", "hello")).await;

    store.delete_session("deleted").await.unwrap();
    s.record(Message::user("u2", "still here")).await;
    s.record(assistant("a2", "ok")).await;

    assert_eq!(stored_ids(&store, "deleted").await, ["u2", "a2"]);
}
