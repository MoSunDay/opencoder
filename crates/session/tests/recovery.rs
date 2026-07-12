//! P3 functional tests for session recovery.
//!
//! Contracts:
//! - resume_reconstructs_history: messages persisted by a run reload byte-equal
//! - continue_picks_latest: --continue logic selects the newest session
//! - fork_does_not_mutate_parent: a copied history leaves the original intact
//! - cross_process_resume: a second store handle (simulating a new process)
//!   reads everything the first wrote
//! - persistence_on_runner_path: a headless-style run with a store attached
//!   writes the user + assistant + tool messages durably
//! - title generation writes the title via small_model

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{generate_title, resume, run, SessionState};
use opencoder_store::{LibsqlStore, Store};

async fn mem_store() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

fn config(model: &str) -> Config {
    Config {
        model: model.into(),
        ..Config::default()
    }
}

fn client_done(text: &str) -> Arc<MockChatClient> {
    Arc::new(MockChatClient::new().with_default(vec![done_event(text)]))
}

fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
        }),
    }
}

async fn run_with_store(
    store: Arc<dyn Store>,
    id: &str,
    model: &str,
    prompt: &str,
    mock: Arc<dyn ChatStream>,
) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(id, agent, config(model), mock, dir.path().to_path_buf())
        .with_store(store);
    run(&mut s, prompt.into(), |_| {}).await.unwrap();
    s
}

#[tokio::test]
async fn resume_reconstructs_history_byte_equal() {
    let store: Arc<dyn Store> = Arc::new(mem_store().await);
    let mock = client_done("assistant-reply");
    let original = run_with_store(
        store.clone(),
        "sess-A",
        "main/glm-5.2",
        "hello world",
        mock.clone() as Arc<dyn ChatStream>,
    )
    .await;

    // simulate a NEW process opening the SAME store and resuming
    let resumed = resume(
        store.clone(),
        "sess-A",
        config("main/glm-5.2"),
        client_done("x") as Arc<dyn ChatStream>,
        original.working_dir.clone(),
    )
    .await
    .unwrap();

    assert_eq!(resumed.id, "sess-A");
    assert_eq!(
        resumed.messages.len(),
        original.messages.len(),
        "history length must match"
    );
    for (a, b) in original.messages.iter().zip(resumed.messages.iter()) {
        assert_eq!(a.role, b.role);
        assert_eq!(a.text(), b.text(), "text must round-trip byte-equal");
        assert_eq!(a.agent, b.agent);
        assert_eq!(a.model, b.model);
        assert_eq!(a.synthetic, b.synthetic);
    }
    assert_eq!(resumed.model, "glm-5.2", "model restored from stored meta");
}

#[tokio::test]
async fn continue_picks_latest_session() {
    let store: Arc<dyn Store> = Arc::new(mem_store().await);
    run_with_store(
        store.clone(),
        "older",
        "m",
        "old prompt",
        client_done("o") as Arc<dyn ChatStream>,
    )
    .await;
    // tiny delay so created_at differs
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    run_with_store(
        store.clone(),
        "newer",
        "m",
        "new prompt",
        client_done("n") as Arc<dyn ChatStream>,
    )
    .await;

    let list = store.list_sessions(&Default::default()).await.unwrap();
    assert_eq!(list[0].id, "newer", "list is newest-first");
    // emulate --continue: take the head of the list
    let picked = list.first().unwrap().id.clone();
    assert_eq!(picked, "newer");
}

#[tokio::test]
async fn cross_process_resume_reads_full_history() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cp.db");

    // process 1: open file store, run a session
    {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open(&db_path).await.unwrap());
        run_with_store(
            store,
            "cp-sess",
            "m",
            "persisted across processes",
            client_done("ok") as Arc<dyn ChatStream>,
        )
        .await;
    }
    // process 2: reopen the SAME file, resume
    let store2: Arc<dyn Store> = Arc::new(LibsqlStore::open(&db_path).await.unwrap());
    let resumed = resume(
        store2.clone(),
        "cp-sess",
        config("m"),
        client_done("y") as Arc<dyn ChatStream>,
        std::path::PathBuf::from("/tmp"),
    )
    .await
    .unwrap();
    let texts: Vec<String> = resumed.messages.iter().map(|m| m.text()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("persisted across processes")),
        "must find the original user msg"
    );
    assert!(
        texts.iter().any(|t| t == "ok"),
        "must find the original assistant reply"
    );
}

#[tokio::test]
async fn persistence_on_runner_path_writes_all_roles() {
    let store: Arc<dyn Store> = Arc::new(mem_store().await);
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "I will read a file".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "tu1".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                }],
                usage: None,
            }])
            // second turn: no tools → done
            .push_script(vec![done_event("done")]),
    ) as Arc<dyn ChatStream>;

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "persist-all",
        agent,
        config("m"),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    run(&mut s, "do a read".into(), |_| {}).await.unwrap();

    let on_disk = store.load_messages("persist-all").await.unwrap();
    let roles: Vec<Role> = on_disk.iter().map(|m| m.role).collect();
    assert!(roles.contains(&Role::User), "user msg persisted");
    assert!(roles.contains(&Role::Assistant), "assistant msg persisted");
    assert!(roles.contains(&Role::Tool), "tool result msg persisted");
    // the assistant block with a ToolUse must round-trip
    let has_tool_use = on_disk.iter().any(|m| {
        m.blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    });
    assert!(has_tool_use, "ToolUse block persisted");
}

#[tokio::test]
async fn title_generation_writes_via_small_model() {
    let store: Arc<dyn Store> = Arc::new(mem_store().await);
    let mock = client_done("body");
    let mut s = run_with_store(
        store.clone(),
        "t",
        "main/glm",
        "build me a snake game",
        mock.clone() as Arc<dyn ChatStream>,
    )
    .await;

    // small-model title call: capture request, return a short title
    let title_mock =
        Arc::new(MockChatClient::new().push_script(vec![done_event("Snake Game Builder")]));
    s.client = title_mock.clone() as Arc<dyn ChatStream>;
    s.config.small_model = Some("cheap/mini".into());
    generate_title(&s).await;

    let reqs = title_mock.requests();
    assert_eq!(reqs[0].model, "mini", "title gen must use small_model id");
    let meta = store.get_session("t").await.unwrap().unwrap();
    assert_eq!(
        meta.title.as_deref(),
        Some("Snake Game Builder"),
        "title persisted to store"
    );
}

#[tokio::test]
async fn fork_does_not_mutate_parent() {
    let store: Arc<dyn Store> = Arc::new(mem_store().await);
    let parent = run_with_store(
        store.clone(),
        "parent",
        "m",
        "original task",
        client_done("p0") as Arc<dyn ChatStream>,
    )
    .await;

    // fork: copy parent's messages into a NEW session id, then run more on the child
    let child_msgs = parent.messages.clone();
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "child".into(),
            title: Some("forked".into()),
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
    store.append_messages("child", &child_msgs).await.unwrap();

    // run the child forward
    let mut child = resume(
        store.clone(),
        "child",
        config("m"),
        client_done("child-extra"),
        parent.working_dir.clone(),
    )
    .await
    .unwrap();
    run(&mut child, "extend the task".into(), |_| {})
        .await
        .unwrap();

    // parent must be untouched
    let parent_disk = store.load_messages("parent").await.unwrap();
    let child_disk = store.load_messages("child").await.unwrap();
    assert!(
        child_disk.len() > parent_disk.len(),
        "child grew beyond parent"
    );
    assert_eq!(
        parent_disk.last().map(|m| m.text()),
        Some("p0".to_string()),
        "parent's last msg unchanged"
    );
}
