//! Integration test for the `@` file-mention feature (TUI worker path):
//! a prompt carrying hand-typed `@relative/path` mention tokens is
//! submitted through `process_cmd(UiCmd::Prompt, …)`; the runner's mention
//! expansion (`opencoder_session::mention_resolve`) rewrites resolvable
//! tokens to ABSOLUTE paths in the recorded user message and the model
//! request, while non-path tokens (emails, unknown names) stay literal.
//! The second test drives the picker key path itself — the production
//! `@`-trigger predicate (`key_handler::char_opens_file_menu`), the real
//! `FileMenu` filter/pick (`file_menu::handle_file_key`), and the composer
//! insert helper — so a pick that drops its `@` marker (tokens silently stop
//! expanding) fails here, not just in unit tests.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::composer;
use opencoder_tui::file_menu::{handle_file_key, FileMenu, FileOutcome};
use opencoder_tui::key_handler::char_opens_file_menu;
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
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(|s| s.to_string())
        .collect()
}

#[tokio::test]
async fn submitted_mentions_record_absolute_paths() {
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join("notes.md"), "notes").unwrap();
    std::fs::create_dir_all(workdir.path().join("src")).unwrap();
    std::fs::write(workdir.path().join("src/main.rs"), "fn main() {}").unwrap();
    let abs = workdir.path().canonicalize().unwrap();

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

    // Hand-typed mentions (`@notes.md`, `@src/main.rs`) — the same token
    // form the picker pins (`@relative/path `); both ride the submit-time
    // expansion, non-path tokens stay verbatim.
    let prompt = "read @notes.md and @src/main.rs, mail a@b.com, see @nope.txt";
    let quit = process_cmd(UiCmd::Prompt(prompt.into(), vec![]), &mut sess, &tx).await;
    assert!(!quit, "Prompt must not break the worker loop");

    // In-memory recorded message: mentions expanded, the rest verbatim.
    let texts: Vec<String> = sess
        .messages
        .iter()
        .filter(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text())
        .collect();
    let want = format!(
        "read {}/notes.md and {}/src/main.rs, mail a@b.com, see @nope.txt",
        abs.display(),
        abs.display()
    );
    assert_eq!(texts, vec![want.clone()], "recorded user message");

    // The model request mirrors it.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(user_contents(&reqs[0]), vec![want.clone()]);

    // And the store carries the same absolute-path text.
    let stored = store.load_messages("mention-flow").await.unwrap();
    let stored_user: Vec<&opencoder_core::Message> =
        stored.iter().filter(|m| m.role == Role::User).collect();
    assert!(
        stored_user.iter().any(|m| m.text() == want),
        "stored user message must carry absolute paths"
    );
}

/// Key-path e2e: the production `@`-trigger predicate opens the picker, a
/// filtered `Enter` pick pins the token through the same helpers the key
/// handler uses, and submitting that composer text records and sends the
/// ABSOLUTE path. Guards the regression where the pick dropped its `@`
/// marker and picker-pinned tokens silently stopped expanding while unit
/// tests stayed green.
#[tokio::test]
async fn picker_key_path_pins_expandable_mention() {
    let workdir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workdir.path().join("src")).unwrap();
    std::fs::write(workdir.path().join("src/main.rs"), "fn main() {}").unwrap();
    let abs = workdir.path().canonicalize().unwrap();

    // Keystroke side: '@' at a token start opens the picker; a mid-token
    // '@' (email position) does not.
    let mut input = String::from("read ");
    let cursor = input.chars().count();
    assert!(char_opens_file_menu(&input, cursor, '@'));
    assert!(!char_opens_file_menu("a", 1, '@'));

    let mut slot = Some(FileMenu::new(workdir.path()));
    for ch in "main".chars() {
        handle_file_key(
            &mut slot,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    assert_eq!(
        slot.as_ref().unwrap().visible_count(),
        1,
        "filter isolates src/main.rs"
    );
    let token = match handle_file_key(&mut slot, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    {
        FileOutcome::Pick(t) => t,
        other => panic!("expected a pick, got {other:?}"),
    };
    assert!(slot.is_none(), "pick closes the menu");
    // Composer keeps the short `@relative/path ` form (trailing space).
    assert_eq!(token, "@src/main.rs ");
    let (s, _i) = composer::insert_str(&input, cursor, &token);
    input = s;
    assert_eq!(input, "read @src/main.rs ");

    // Submit side: same worker path as the hand-typed case.
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "mention-keypath".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new().push_script(vec![text_done("ok")]));
    let (tx, _rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "mention-keypath",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        mock.clone(),
        workdir.path().to_path_buf(),
    )
    .with_store(store.clone());

    let quit = process_cmd(UiCmd::Prompt(input, vec![]), &mut sess, &tx).await;
    assert!(!quit, "Prompt must not break the worker loop");

    // Recorded message and model request carry the absolute path.
    let want = format!("read {}/src/main.rs ", abs.display());
    let texts: Vec<String> = sess
        .messages
        .iter()
        .filter(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text())
        .collect();
    assert_eq!(texts, vec![want.clone()], "recorded user message");
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(user_contents(&reqs[0]), vec![want]);
}
