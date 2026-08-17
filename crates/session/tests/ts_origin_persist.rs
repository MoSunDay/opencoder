//! Integration test: ts-origin sessions persist `model: None` / `agent: None`
//! as the durable ownership marker, and the session row is created **lazily**
//! (only on first `record`), matching the contract the tmux `ts` flow relies
//! on for `ts -l` filtering.

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn mock_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>
}

fn make_session(id: &str, store: Arc<dyn Store>, ts_origin: bool) -> SessionState {
    let mut s = SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock_client(),
        PathBuf::from("."),
    )
    .with_store(store);
    if ts_origin {
        s = s.ts_origin();
    }
    s
}

/// Before any `record()`, no session row exists in the store — the row is
/// created lazily on first message, not eagerly at construction.
#[tokio::test]
async fn ts_origin_no_row_before_first_record() {
    let store = mem_store().await;
    let _s = make_session("ts-lazy", store.clone(), true);
    assert!(store.get_session("ts-lazy").await.unwrap().is_none());
    assert_eq!(store.load_messages("ts-lazy").await.unwrap().len(), 0);
}

/// After the first `record()`, the session row exists with `model: None` and
/// `agent: None` — the ts-ownership marker.
#[tokio::test]
async fn ts_origin_persists_null_model_and_agent() {
    let store = mem_store().await;
    let mut s = make_session("ts-null", store.clone(), true);

    s.record(Message::user("u1", "hello world")).await;

    let meta = store
        .get_session("ts-null")
        .await
        .unwrap()
        .expect("session row should exist after record");
    assert_eq!(
        meta.model, None,
        "ts-origin session must persist model: None"
    );
    assert_eq!(
        meta.agent, None,
        "ts-origin session must persist agent: None"
    );
    assert_eq!(
        meta.title.as_deref(),
        Some("hello world"),
        "title should be derived from first user text"
    );
    assert_eq!(store.load_messages("ts-null").await.unwrap().len(), 1);
}

/// Without ts_origin (normal TUI/run path), model and agent are populated.
#[tokio::test]
async fn normal_session_persists_model_and_agent() {
    let store = mem_store().await;
    let mut s = make_session("normal", store.clone(), false);

    s.record(Message::user("u1", "do something")).await;

    let meta = store
        .get_session("normal")
        .await
        .unwrap()
        .expect("session row should exist after record");
    assert_eq!(meta.model.as_deref(), Some("m/g"));
    assert_eq!(meta.agent.as_deref(), Some("act"));
}

/// A resumed ts-origin session keeps `session_created = true` and does not
/// create a duplicate row or overwrite the model on subsequent records.
#[tokio::test]
async fn ts_origin_resume_keeps_null_model() {
    let store = mem_store().await;

    // Phase 1: ts-origin session records a message.
    {
        let mut s = make_session("ts-resume", store.clone(), true);
        s.record(Message::user("u1", "first prompt")).await;
    }

    // Phase 2: resume from store.
    let mut resumed = opencoder_session::resume::resume(
        store.clone(),
        "ts-resume",
        Config {
            model: "different/model".into(),
            ..Config::default()
        },
        mock_client(),
        PathBuf::from("."),
    )
    .await
    .unwrap();

    let meta = store.get_session("ts-resume").await.unwrap().unwrap();
    assert_eq!(meta.model, None, "resumed ts-origin keeps model: None");

    // Recording another message must NOT create a new row or clobber model.
    resumed.record(Message::user("u2", "second prompt")).await;
    assert_eq!(store.load_messages("ts-resume").await.unwrap().len(), 2);

    let meta2 = store.get_session("ts-resume").await.unwrap().unwrap();
    assert_eq!(
        meta2.model, None,
        "model stays None after post-resume record"
    );
}
