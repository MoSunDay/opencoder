//! Integration test: `@path` mentions are expanded to absolute paths at all
//! three runner entry points — direct prompt, steer promotion, and queue
//! drain — so the RECORDED user message (and the model request) carries the
//! full path while non-path tokens stay literal.
//!
//! The pure unit tests live in `src/mention_resolve.rs`; this file drives
//! the real runner (`run`) so both hooks (`resolve_inline_skills` tail and
//! `record_compound` head) are exercised end-to-end.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{run, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Create the session row so FK-backed input admission succeeds.
async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Admit a store-backed input (steer or queue) for `session_id`.
async fn admit(
    store: &Arc<dyn Store>,
    session_id: &str,
    id: &str,
    prompt: &str,
    delivery: Delivery,
) {
    store
        .admit_input(&SessionInput {
            seq: None,
            id: id.into(),
            session_id: session_id.into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();
}

/// Act session wired to a store + mock client, rooted in a temp workdir
/// containing `notes.md` and `src/main.rs`.
fn workdir_session(
    store: Arc<dyn Store>,
    mock: Arc<MockChatClient>,
    id: &str,
) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.md"), "notes").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
    let s = SessionState::new(
        id,
        resolve_agent("act").expect("act agent"),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store)
    .mark_session_created();
    (dir, s)
}

fn user_texts(session: &SessionState) -> Vec<String> {
    session
        .messages
        .iter()
        .filter(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text())
        .collect()
}

/// Injection point 1 — direct prompt: the recorded kickoff user message has
/// the `@notes.md` token rewritten to the absolute path, while `@nope.txt`
/// and the email stay literal.
#[tokio::test]
async fn direct_prompt_expands_mentions() {
    let store = mem_store().await;
    seed(&store, "direct-mention").await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (dir, mut s) = workdir_session(store, mock.clone(), "direct-mention");

    run(
        &mut s,
        "read @notes.md not @nope.txt mail a@b.com".into(),
        |_| {},
    )
    .await
    .unwrap();

    let abs = dir.path().canonicalize().unwrap();
    assert_eq!(
        user_texts(&s),
        vec![format!(
            "read {}/notes.md not @nope.txt mail a@b.com",
            abs.display()
        )],
        "recorded user message must carry the absolute path only"
    );
    // The model request mirrors the recorded message.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    let want = format!("{}/notes.md", abs.display());
    assert!(
        reqs[0]
            .messages
            .iter()
            .any(|m| m.to_string().contains(&want)),
        "request must carry the absolute path"
    );
    assert!(
        !reqs[0]
            .messages
            .iter()
            .any(|m| m.to_string().contains("@notes.md")),
        "raw token must be gone"
    );
}

/// Injection point 2 — steer promotion: a steer admitted before the run is
/// promoted at the first turn boundary with mentions already expanded.
#[tokio::test]
async fn steer_prompt_expands_mentions() {
    let store = mem_store().await;
    seed(&store, "steer-mention").await;
    admit(
        &store,
        "steer-mention",
        "steer-1",
        "also open @src/main.rs",
        Delivery::Steer,
    )
    .await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("r1")]));
    let (dir, mut s) = workdir_session(store, mock, "steer-mention");

    run(&mut s, "kickoff".into(), |_| {}).await.unwrap();

    let abs = dir.path().canonicalize().unwrap();
    let texts = user_texts(&s);
    assert_eq!(texts.len(), 2, "kickoff + one promoted steer");
    assert_eq!(
        texts[1],
        format!("also open {}/src/main.rs", abs.display()),
        "promoted steer must carry the absolute path"
    );
}

/// Injection point 3 — idle queue drain: a queued follow-up is drained after
/// the kickoff turn and recorded with the expanded absolute path.
#[tokio::test]
async fn queued_prompt_expands_mentions() {
    let store = mem_store().await;
    seed(&store, "queue-mention").await;
    admit(
        &store,
        "queue-mention",
        "queue-1",
        "check @notes.md please",
        Delivery::Queue,
    )
    .await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff reply")])
            .push_script(vec![done_turn("queued reply")]),
    );
    let (dir, mut s) = workdir_session(store, mock.clone(), "queue-mention");

    run(&mut s, "kickoff".into(), |_| {}).await.unwrap();

    let abs = dir.path().canonicalize().unwrap();
    let texts = user_texts(&s);
    assert_eq!(texts.len(), 2, "kickoff + one drained queue item");
    assert_eq!(
        texts[1],
        format!("check {}/notes.md please", abs.display()),
        "drained queue item must carry the absolute path"
    );
    assert_eq!(mock.requests().len(), 2, "kickoff turn + drained turn");
}
