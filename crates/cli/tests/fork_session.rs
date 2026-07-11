use std::sync::Arc;

use opencode_cli::run::fork_session;
use opencode_core::{ContentBlock, Message};
use opencode_store::{LibsqlStore, SessionMeta, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("parent".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();
    store
        .append_message(id, &Message::user("u1", "hello"))
        .await
        .unwrap();
    store
        .append_message(id, &assistant_with_text("a1", "world"))
        .await
        .unwrap();
}

#[tokio::test]
async fn fork_copies_messages_and_leaves_original_intact() {
    let store = mem_store().await;
    seed(&store, "parent").await;

    let child_id = fork_session(store.as_ref(), "parent").await.unwrap();
    assert_ne!(child_id, "parent", "fork must create a new id");

    let parent_msgs = store.load_messages("parent").await.unwrap();
    let child_msgs = store.load_messages(&child_id).await.unwrap();
    assert_eq!(parent_msgs.len(), 2, "parent unchanged");
    assert_eq!(child_msgs.len(), 2, "child has same message count");
    assert_eq!(
        child_msgs[0].text(),
        parent_msgs[0].text(),
        "child copies parent content"
    );

    let child_meta = store.get_session(&child_id).await.unwrap().unwrap();
    assert_eq!(child_meta.title.as_deref(), Some("parent (fork)"));
    assert_eq!(child_meta.model.as_deref(), Some("m"));
}

#[tokio::test]
async fn fork_creates_independent_sessions() {
    let store = mem_store().await;
    seed(&store, "orig").await;

    let child = fork_session(store.as_ref(), "orig").await.unwrap();

    store
        .append_message(&child, &Message::user("u2", "extra turn"))
        .await
        .unwrap();

    let orig_msgs = store.load_messages("orig").await.unwrap();
    let child_msgs = store.load_messages(&child).await.unwrap();
    assert_eq!(orig_msgs.len(), 2, "parent untouched by child mutation");
    assert_eq!(child_msgs.len(), 3, "child grew");
    assert_eq!(child_msgs[2].text(), "extra turn");
}

#[tokio::test]
async fn fork_nonexistent_session_errors() {
    let store = mem_store().await;
    let err = fork_session(store.as_ref(), "ghost").await;
    assert!(err.is_err(), "forking a nonexistent session should fail");
}
