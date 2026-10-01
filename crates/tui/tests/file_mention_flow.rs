use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

fn user_contents(req: &opencoder_llm::ChatRequest) -> Vec<String> {
    opencoder_llm::lower_messages(&req.messages)
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(|s| s.to_string())
        .collect()
}

#[tokio::test]
async fn tui_at_paths_are_literal_in_messages_requests_and_store() {
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join("notes.md"), "notes").unwrap();
    std::fs::create_dir_all(workdir.path().join("src")).unwrap();
    std::fs::write(workdir.path().join("src/main.rs"), "fn main() {}").unwrap();

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "mention-flow".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("ok")]));
    let (tx, _rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "mention-flow",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        mock.clone(),
        workdir.path().to_path_buf(),
    )
    .with_store(store.clone());

    // Hand-typed @ paths remain literal through the submit-time
    // expansion, non-path tokens stay verbatim.
    let prompt = "read @notes.md and @src/main.rs, mail a@b.com, see @nope.txt";
    let quit = process_cmd(UiCmd::Prompt(prompt.into(), vec![]), &mut sess, &tx).await;
    assert!(!quit, "Prompt must not break the worker loop");

    // In-memory recorded message: mentions literal, the rest verbatim.
    let texts: Vec<String> = sess
        .messages
        .iter()
        .filter(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text())
        .collect();
    let want = prompt.to_owned();
    assert_eq!(texts, vec![want.clone()], "recorded user message");

    // The model request mirrors it.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(user_contents(&reqs[0]), vec![want.clone()]);

    // And the store carries the same literal text.
    let stored = store.load_messages("mention-flow").await.unwrap();
    let stored_user: Vec<&opencoder_core::Message> =
        stored.iter().filter(|m| m.role == Role::User).collect();
    assert!(
        stored_user.iter().any(|m| m.text() == want),
        "stored user message must carry literal @ paths"
    );
}
