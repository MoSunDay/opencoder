//! Regression tests for the mouse wheel-scroll handler, focusing on the
//! bug where `ScrollDown` computed `max_rows` from the PARENT chat even
//! while a subagent perspective was focused — pinning to the bottom and
//! making the child body un-scrollable.
use super::*;
use opencoder_store::SessionMeta;

mod mouse_tests;

#[test]
fn paste_existing_absolute_file_echoes_full_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let raw = tmp.path().to_string_lossy().into_owned();
    let expected = tmp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    // Absolute paths ignore workdir.
    assert_eq!(paste_payload(&raw, Path::new("/")), expected);
}

#[test]
fn paste_existing_file_with_trailing_newline_echoes_full_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let raw = tmp.path().to_string_lossy().into_owned();
    let expected = tmp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(paste_payload(&format!("{raw}\n"), Path::new("/")), expected);
}

#[test]
fn paste_quoted_absolute_file_echoes_full_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let raw = tmp.path().to_string_lossy().into_owned();
    let expected = tmp
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(paste_payload(&format!("'{raw}'"), Path::new("/")), expected);
    assert_eq!(paste_payload(&format!("{raw}\""), Path::new("/")), expected);
}

#[test]
fn paste_existing_relative_file_resolves_against_workdir() {
    // A drag-pasted bare relative filename resolves to its full absolute
    // path when it exists relative to the session workdir.
    let dir = tempfile::tempdir().unwrap();
    let rel = "src/main.rs";
    let abs = dir.path().join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, "fn main(){}").unwrap();
    let expected = abs.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(paste_payload(rel, dir.path()), expected);
}

#[test]
fn paste_nonexistent_absolute_path_returned_verbatim() {
    let raw = "/this/does/not/exist/xyz";
    assert_eq!(paste_payload(raw, Path::new("/")), raw);
}

#[test]
fn paste_multiline_text_returned_verbatim() {
    let raw = "first line\nsecond line\n";
    assert_eq!(paste_payload(raw, Path::new("/")), raw);
}

#[test]
fn paste_empty_returned_verbatim() {
    assert_eq!(paste_payload("", Path::new("/")), "");
    assert_eq!(paste_payload("\n", Path::new("/")), "\n");
}

#[test]
fn paste_non_file_text_returned_verbatim() {
    // A plain word that is not an existing file relative to workdir is
    // never rewritten, so ordinary text pastes are never surprising.
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(paste_payload("hello world", dir.path()), "hello world");
}

#[tokio::test]
async fn clear_pending_inputs_drops_store_rows_and_mirrors() {
    use opencoder_store::LibsqlStore;
    let store = LibsqlStore::open_memory().await.unwrap();
    let sid = "s1";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let s_seq = store
        .admit_input(&mk_input_with_images(sid, Delivery::Steer, "steer-1", &[]))
        .await
        .unwrap();
    let q_seq = store
        .admit_input(&mk_input_with_images(sid, Delivery::Queue, "queue-1", &[]))
        .await
        .unwrap();
    let mut steer_items = vec![(s_seq, String::from("steer-1"))];
    let mut queue_items = vec![(q_seq, String::from("queue-1"))];

    clear_pending_inputs(&store, &mut steer_items, &mut queue_items).await;

    assert!(steer_items.is_empty(), "steer mirror cleared");
    assert!(queue_items.is_empty(), "queue mirror cleared");
    assert!(
        store
            .pending_inputs(sid, Delivery::Steer)
            .await
            .unwrap()
            .is_empty(),
        "steer rows deleted from store"
    );
    assert!(
        store
            .pending_inputs(sid, Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "queue rows deleted from store"
    );
}

/// Ctrl+U is now a pure act<->plan mode toggle and must NOT be consumed by
/// `pre_key_intercept` (so it falls through to `handle_key`, which switches
/// mode without collapsing thinking or clearing the input). Ctrl+L still owns
/// the collapse/clear behaviour.
#[test]
fn ctrl_u_not_intercepted_ctrl_l_clears_input() {
    fn run(key: KeyEvent) -> (bool, String, usize) {
        let mut chat = ChatView::default();
        let mut subagent_focus: Option<usize> = None;
        let mut scroll = 5u32;
        let mut follow = true;
        let mut selection = None;
        let mut last_esc = None;
        let mut input = "hello world".to_string();
        let mut cursor = 5usize;
        let consumed = pre_key_intercept(
            key,
            &mut subagent_focus,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut last_esc,
            &mut chat,
            &mut input,
            &mut cursor,
            0,
            true,
        );
        (consumed, input, cursor)
    }

    let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);

    // Ctrl+U must pass through untouched (handled downstream as a mode toggle).
    let (u_consumed, u_input, u_cursor) = run(ctrl_u);
    assert!(
        !u_consumed,
        "Ctrl+U must NOT be consumed by pre_key_intercept"
    );
    assert_eq!(
        u_input, "hello world",
        "Ctrl+U must leave the input untouched"
    );
    assert_eq!(u_cursor, 5, "Ctrl+U must not move the cursor");

    // Ctrl+L still collapses thinking / clears the input.
    let (l_consumed, l_input, l_cursor) = run(ctrl_l);
    assert!(l_consumed, "Ctrl+L must be consumed by pre_key_intercept");
    assert!(l_input.is_empty(), "Ctrl+L must clear the input");
    assert_eq!(l_cursor, 0, "Ctrl+L must reset the cursor");
}

#[test]
fn mk_input_with_images_passes_images_through() {
    let images = vec!["data:image/png;base64,abc".to_string()];
    let input = crate::app_helpers::mk_input_with_images(
        "s1",
        opencoder_store::Delivery::Steer,
        "hello",
        &images,
    );
    assert_eq!(input.session_id, "s1");
    assert_eq!(input.prompt, "hello");
    assert_eq!(input.images, images);
    assert_eq!(input.delivery, opencoder_store::Delivery::Steer);
}

#[test]
fn drain_pending_images_collects_and_clears() {
    let mut pending: Vec<(String, String)> = vec![
        ("data:image/png;base64,AAA".into(), "a.png".into()),
        ("data:image/png;base64,BBB".into(), "b.png".into()),
    ];
    let uris = crate::app_helpers::drain_pending_images(&mut pending);
    assert_eq!(
        uris,
        vec![
            "data:image/png;base64,AAA".to_string(),
            "data:image/png;base64,BBB".to_string(),
        ]
    );
    assert!(
        pending.is_empty(),
        "pending buffer must be cleared after drain"
    );
}

#[test]
fn drain_pending_images_empty_yields_empty() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let uris = crate::app_helpers::drain_pending_images(&mut pending);
    assert!(uris.is_empty());
    assert!(pending.is_empty());
}

#[test]
fn mk_input_with_images_defaults_empty_when_none() {
    let input = crate::app_helpers::mk_input_with_images(
        "s2",
        opencoder_store::Delivery::Queue,
        "plain",
        &[],
    );
    assert!(input.images.is_empty());
}

#[test]
fn reapply_session_model_overrides_resumed_model() {
    use opencoder_core::resolve_agent;
    use opencoder_llm::{ChatStream, MockChatClient};
    use std::path::PathBuf;
    // A session "resumed" with a stored model of gpt-4o-mini.
    let cfg = Config {
        model: "openai/gpt-4o-mini".into(),
        ..Config::default()
    };
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "s1",
        agent,
        cfg,
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn ChatStream>,
        PathBuf::from("/tmp"),
    );
    // Explicit --model wins over the stored model; returns the value to persist.
    let changed = reapply_session_model(&mut s, &Some("anthropic/claude-3".into()));
    assert_eq!(changed.as_deref(), Some("anthropic/claude-3"));
    assert_eq!(s.model, "claude-3");
    assert_eq!(s.config.provider_id(), "anthropic");
    // No-op when the override already matches or is absent.
    assert_eq!(reapply_session_model(&mut s, &Some("anthropic/claude-3".into())), None);
    assert_eq!(reapply_session_model(&mut s, &None), None);
}

#[tokio::test]
async fn open_store_creates_db_file_in_workdir_hashed_data_dir() {
    // `open_store` materializes the on-disk sqlite DB at
    // `<data_local>/opencoder/<hash(workdir)>/opencoder.db`. A tempdir workdir
    // hashes to a unique subdir, so it is isolated and safe to scrub. This
    // guards the extraction (moved verbatim out of `app::run`) by asserting the
    // observable side effect — the DB file is created — without needing a live
    // terminal or a running session.
    let workdir = tempfile::tempdir().expect("tempdir");
    let data_dir = data_dir_for(workdir.path());
    // Defensive: clear any stale subdir from a prior identical-hash run.
    let _ = std::fs::remove_dir_all(&data_dir);

    let store = open_store(workdir.path()).await.expect("open_store succeeds");
    let db_file = data_dir.join("opencoder.db");
    assert!(
        db_file.exists(),
        "open_store must create opencoder.db at {}",
        db_file.display()
    );

    // Release the connection so we can clean up every artifact we created
    // (db file + any -wal/-shm sidecars) without holding a lock.
    drop(store);
    let _ = std::fs::remove_dir_all(&data_dir);
}
