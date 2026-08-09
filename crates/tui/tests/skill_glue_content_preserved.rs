//! Regression: a `$skill` token glued to user content (e.g. `$review1) task`)
//! must not silently delete the glued digits/text. The greedy `[a-z0-9-]`
//! charset scans `review1` as the token name, but since `review1` is not a
//! real skill it is *unresolved* and must be preserved verbatim — the user's
//! numbered list `1) ... 2) ...` reaches the model intact.
//!
//! Before the fix, `extract_skill_tokens` stripped *all* tokens unconditionally,
//! so `$review1` vanished and the model saw `) task` with the `1` gone.
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
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

/// Collect every user-role message content from a ChatRequest.
fn user_contents(req: &opencoder_llm::ChatRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(String::from))
        .collect()
}

#[tokio::test]
async fn glued_skill_token_preserves_numbered_list() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "glue".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("done")]));
    let (tx, _rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "glue",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        mock.clone(),
        std::env::temp_dir(),
    )
    .with_store(store);

    // `$review1)` — the picker inserts `$review ` (space-separated), but a user
    // may also type the token directly and glue it to content. Either way the
    // greedy scanner reads `review1`, which resolves to no real skill, so the
    // entire `$review1` must survive as literal text.
    let prompt = "$review1) \u{4fee}\u{767b}\u{5f55}bug\n2) \u{52a0}\u{6d4b}\u{8bd5}";
    let quit = process_cmd(UiCmd::Prompt(prompt.into(), Vec::new()), &mut sess, &tx).await;
    assert!(!quit);

    let requests = mock.requests();
    assert!(!requests.is_empty(), "model must have been called");

    let users = user_contents(&requests[0]);
    let joined = users.join("\n");
    assert!(
        joined.contains("1)"),
        "the `1)` list item must survive token parsing: {joined:?}"
    );
    assert!(
        joined.contains("2)"),
        "the `2)` list item must survive token parsing: {joined:?}"
    );
}
