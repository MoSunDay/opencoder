//! Regression tests for the mouse wheel-scroll handler, focusing on the
//! bug where `ScrollDown` computed `max_rows` from the PARENT chat even
//! while a subagent perspective was focused — pinning to the bottom and
//! making the child body un-scrollable.
use super::*;
use opencoder_store::SessionMeta;

mod ctrl_l_tests;
mod mouse_clip_tests;
mod mouse_dbl_click_tests;
mod mouse_helpers;
mod mouse_scroll_tests;
mod mouse_tests;
mod mouse_wheel_tests;

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
        .admit_input(&mk_input_with_images(
            sid,
            Delivery::Steer,
            "steer-1",
            None,
            &[],
        ))
        .await
        .unwrap();
    let q_seq = store
        .admit_input(&mk_input_with_images(
            sid,
            Delivery::Queue,
            "queue-1",
            None,
            &[],
        ))
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

/// Ctrl+T is now a pure act<->plan mode toggle and must NOT be consumed by
/// `pre_key_intercept` (so it falls through to `handle_key`, which switches
/// mode without collapsing thinking or clearing the input). Ctrl+L owns the
/// collapse/clear/follow behaviour (without the forced redraw — that moved to
/// Ctrl+F).
#[test]
fn ctrl_t_not_intercepted_ctrl_l_clears_ctrl_f_redraws() {
    fn run(key: KeyEvent) -> (bool, String, usize, bool, bool) {
        let mut chat = ChatView::default();
        let mut subagent_focus: Option<usize> = None;
        let mut follow = false;
        let mut selection = None;
        let mut last_esc = None;
        let mut input = "hello world".to_string();
        let mut cursor = 5usize;
        let mut needs_clear = false;
        let consumed = pre_key_intercept(
            key,
            &mut subagent_focus,
            &mut follow,
            &mut selection,
            &mut last_esc,
            &mut chat,
            &mut input,
            &mut cursor,
            &mut needs_clear,
        );
        (consumed, input, cursor, needs_clear, follow)
    }

    let ctrl_t = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    let ctrl_f = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);

    // Ctrl+T must pass through untouched (handled downstream as a mode toggle).
    let (t_consumed, t_input, t_cursor, t_clear, t_follow) = run(ctrl_t);
    assert!(
        !t_consumed,
        "Ctrl+T must NOT be consumed by pre_key_intercept"
    );
    assert_eq!(
        t_input, "hello world",
        "Ctrl+T must leave the input untouched"
    );
    assert_eq!(t_cursor, 5, "Ctrl+T must not move the cursor");
    assert!(!t_clear, "Ctrl+T must not request a forced clear/redraw");
    assert!(!t_follow, "Ctrl+T must not touch follow mode");

    // Ctrl+L still collapses thinking / clears the input, but no longer
    // forces the full-screen redraw (that is Ctrl+F's job now).
    let (l_consumed, l_input, l_cursor, l_clear, l_follow) = run(ctrl_l);
    assert!(l_consumed, "Ctrl+L must be consumed by pre_key_intercept");
    assert!(l_input.is_empty(), "Ctrl+L must clear the input");
    assert_eq!(l_cursor, 0, "Ctrl+L must reset the cursor");
    assert!(
        !l_clear,
        "Ctrl+L must NOT request a forced full-screen redraw anymore"
    );
    assert!(
        l_follow,
        "Ctrl+L must return to follow mode (bottom of the view)"
    );

    // Ctrl+F: force redraw only — consumes the key, sets needs_clear, and
    // leaves the input / cursor untouched.
    let (f_consumed, f_input, f_cursor, f_clear, f_follow) = run(ctrl_f);
    assert!(f_consumed, "Ctrl+F must be consumed by pre_key_intercept");
    assert_eq!(
        f_input, "hello world",
        "Ctrl+F must leave the input untouched"
    );
    assert_eq!(f_cursor, 5, "Ctrl+F must not move the cursor");
    assert!(
        f_clear,
        "Ctrl+F must request a forced full-screen redraw (needs_clear == true)"
    );
    assert!(!f_follow, "Ctrl+F must not touch follow mode");
}

#[test]
fn mk_input_with_images_passes_images_through() {
    let images = vec!["data:image/png;base64,abc".to_string()];
    let input = crate::app_helpers::mk_input_with_images(
        "s1",
        opencoder_store::Delivery::Steer,
        "hello",
        Some("$skill hello".to_string()),
        &images,
    );
    assert_eq!(input.session_id, "s1");
    assert_eq!(input.prompt, "hello");
    assert_eq!(input.images, images);
    assert_eq!(input.delivery, opencoder_store::Delivery::Steer);
    assert_eq!(
        input.display_text.as_deref(),
        Some("$skill hello"),
        "display_text must be passed through verbatim"
    );
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
        None,
        &[],
    );
    assert!(input.images.is_empty());
    assert!(
        input.display_text.is_none(),
        "None display_text must stay None (consumers fall back to prompt)"
    );
}

#[test]
fn mk_input_with_images_passes_display_text() {
    let input = crate::app_helpers::mk_input_with_images(
        "s3",
        opencoder_store::Delivery::Queue,
        "clean prompt",
        Some("$repo-memory clean prompt".to_string()),
        &[],
    );
    assert_eq!(
        input.display_text.as_deref(),
        Some("$repo-memory clean prompt"),
        "the display form must be preserved verbatim while prompt stays clean"
    );
    assert_eq!(
        input.prompt, "clean prompt",
        "prompt (LLM contract) unchanged"
    );
}

fn pending_row(
    seq: i64,
    session_id: &str,
    admitted_seq: i64,
    delivery: Delivery,
    prompt: &str,
    display_text: Option<&str>,
) -> SessionInput {
    SessionInput {
        seq: Some(seq),
        id: format!("in-{seq}"),
        session_id: session_id.into(),
        delivery,
        prompt: prompt.into(),
        images: Vec::new(),
        display_text: display_text.map(|d| d.to_string()),
        admitted_seq,
        promoted_seq: None,
    }
}

#[test]
fn pending_mirror_uses_display_text_with_prompt_fallback() {
    let rows = vec![
        pending_row(
            10,
            "s",
            1,
            Delivery::Queue,
            "fix the bug",
            Some("$repo-memory fix the bug"),
        ),
        pending_row(11, "s", 2, Delivery::Steer, "steer me", None),
    ];
    let mirror = crate::queue_panel::pending_mirror(rows);
    assert_eq!(
        mirror,
        vec![
            (10, "$repo-memory fix the bug".to_string()),
            (11, "steer me".to_string()),
        ],
        "display_text verbatim when present; prompt fallback when None"
    );
}

#[tokio::test]
async fn restore_pending_mirrors_restores_display_text_at_reload() {
    use opencoder_store::LibsqlStore;
    let store = LibsqlStore::open_memory().await.unwrap();
    let sid = "resume-sess";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Queued input with a distinct display form (raw `$skill` original).
    let row = pending_row(
        0,
        sid,
        1,
        Delivery::Queue,
        "fix the bug",
        Some("$repo-memory fix the bug"),
    );
    let q_seq = store.admit_input(&row).await.unwrap();
    // Steered input admitted without a display form (pre-display_text rows).
    let s_seq = store
        .admit_input(&pending_row(0, sid, 2, Delivery::Steer, "steer me", None))
        .await
        .unwrap();

    let mut steer_items: Vec<(i64, String)> = Vec::new();
    let store: Arc<dyn Store> = Arc::new(store);
    let queue_items =
        crate::queue_panel::restore_pending_mirrors(&store, sid, &mut steer_items).await;

    assert_eq!(
        queue_items,
        vec![(q_seq, "$repo-memory fix the bug".to_string())],
        "queue mirror restores the display original"
    );
    assert_eq!(
        steer_items,
        vec![(s_seq, "steer me".to_string())],
        "steer mirror falls back to prompt when display_text is None"
    );
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
    assert_eq!(
        reapply_session_model(&mut s, &Some("anthropic/claude-3".into())),
        None
    );
    assert_eq!(reapply_session_model(&mut s, &None), None);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // single-threaded test runtime; lock serializes env-var readers
async fn open_store_creates_db_file_in_workdir_hashed_data_dir() {
    // `open_store` materializes the on-disk sqlite DB at
    // `<data_local>/opencoder/<hash(workdir)>/opencoder.db`. A tempdir workdir
    // hashes to a unique subdir, so it is isolated and safe to scrub. This
    // guards the extraction (moved verbatim out of `app::run`) by asserting the
    // observable side effect — the DB file is created — without needing a live
    // terminal or a running session.
    // Serialize with HOME/XDG env-var mutators: `data_dir_for` resolves the
    // data root via `dirs::data_local_dir()` (reads HOME/XDG_DATA_HOME). A
    // concurrent env mutation in another test can make the computed path
    // inconsistent, causing the assertion to flake under parallel load.
    let _lock = crate::app::app_loop::tests::HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workdir = tempfile::tempdir().expect("tempdir");
    let data_dir = data_dir_for(workdir.path());
    // Defensive: clear any stale subdir from a prior identical-hash run.
    let _ = std::fs::remove_dir_all(&data_dir);

    let store = open_store(workdir.path())
        .await
        .expect("open_store succeeds");
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

#[test]
fn snapshot_image_uris_returns_uris_without_clearing() {
    // Gap 4: snapshot must NOT clear the buffer so images survive a failed
    // store write or dead worker (only cleared on the success path).
    let pending: Vec<(String, String)> = vec![
        ("data:image/png;base64,AAA".into(), "a.png".into()),
        ("data:image/png;base64,BBB".into(), "b.png".into()),
    ];
    let uris = crate::app_helpers::snapshot_image_uris(&pending);
    assert_eq!(
        uris,
        vec![
            "data:image/png;base64,AAA".to_string(),
            "data:image/png;base64,BBB".to_string(),
        ]
    );
    assert_eq!(
        pending.len(),
        2,
        "snapshot must NOT clear the pending buffer"
    );
}

#[test]
fn snapshot_image_uris_empty_yields_empty() {
    let pending: Vec<(String, String)> = Vec::new();
    let uris = crate::app_helpers::snapshot_image_uris(&pending);
    assert!(uris.is_empty());
}

/// Gap 1: when a skill-only submit happens while a turn is running, the skill
/// trigger is admitted as a **queued** input with the snapshotted images, and
/// `pending_images` is cleared only on success. This test exercises the exact
/// sequence used by the `else` branch of the Submit handler in `app.rs`.
#[tokio::test]
async fn skill_only_submit_while_running_drains_images_via_queue() {
    use opencoder_store::Delivery;
    use opencoder_store::LibsqlStore;

    let store = LibsqlStore::open_memory().await.unwrap();
    let sid = "gap1-session";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            task_type: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();

    // Simulate pending images from a paste/drop.
    let mut pending_images: Vec<(String, String)> =
        vec![("data:image/png;base64,AAAA".into(), "img1.png".into())];

    // Step 1: snapshot WITHOUT clearing (images survive a failed admit).
    let skill_name = "my-skill";
    let trigger = crate::skill_display::skill_trigger(skill_name);
    let image_uris = crate::app_helpers::snapshot_image_uris(&pending_images);

    // Step 2: admit as a queued input (mirrors the else branch).
    let input =
        crate::app_helpers::mk_input_with_images(sid, Delivery::Queue, &trigger, None, &image_uris);
    let result = store.admit_input(&input).await;

    // Step 3: on success, clear pending images.
    assert!(result.is_ok(), "admit_input should succeed");
    pending_images.clear();

    // Verify: images drained (not leaked).
    assert!(
        pending_images.is_empty(),
        "pending_images must be cleared after successful queue admit"
    );

    // Verify: the queued input carries the images and skill trigger.
    let inputs = store.pending_inputs(sid, Delivery::Queue).await.unwrap();
    let queued = inputs
        .iter()
        .find(|i| i.delivery == Delivery::Queue)
        .expect("queued input must exist in store");
    assert_eq!(
        queued.prompt, trigger,
        "queued prompt must be the skill trigger"
    );
    assert_eq!(
        queued.images,
        vec!["data:image/png;base64,AAAA".to_string()],
        "queued input must carry the image URI"
    );
}

/// Combined-content submit while running: when a prompt contains BOTH a
/// `$skill` token AND other text, the skill is resolved (body injected into
/// the system prompt) while the **clean text** — not the skill trigger — is what
/// reaches the queue. This is the critical contract for "skill + other input".
#[tokio::test]
async fn combined_skill_and_text_submit_while_running_queues_clean_text() {
    use opencoder_core::extract_skill_tokens;
    use opencoder_store::Delivery;
    use opencoder_store::LibsqlStore;

    let store = LibsqlStore::open_memory().await.unwrap();
    let sid = "combined-session";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            task_type: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();

    // The raw prompt a user typed: a skill token interleaved with prose.
    let raw = "$repo-memory fix the bug in main.rs";
    let (clean, names) = extract_skill_tokens(raw);

    // Precondition: the token parsed and clean text carries the real task.
    assert_eq!(names, vec!["repo-memory"]);
    assert_eq!(clean.trim(), "fix the bug in main.rs");

    // Mirrors the `else if running` branch in app.rs Submit: the *clean* text
    // (not the trigger) is admitted as a queued input.
    let trimmed = clean.trim();
    assert!(
        !trimmed.is_empty(),
        "combined content must not collapse to a pure-skill trigger"
    );
    let input = crate::app_helpers::mk_input_with_images(sid, Delivery::Queue, trimmed, None, &[]);
    store.admit_input(&input).await.unwrap();

    // Verify: the queued prompt is the user's task text, NOT a skill trigger.
    let queued = store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.delivery == Delivery::Queue)
        .expect("queued input must exist");
    assert_eq!(
        queued.prompt, "fix the bug in main.rs",
        "combined-content submit must queue the clean task text"
    );
    assert!(
        !queued.prompt.contains("skill is now active"),
        "the skill trigger must NOT be queued when there is other text"
    );
}

// ---------------------------------------------------------------------------
// apply_force_redraw — Ctrl+F force-redraw helper
// ---------------------------------------------------------------------------

/// Concatenate every cell symbol of a ratatui buffer into one searchable
/// string. Mirrors the helper in `render_clear_tests.rs`; used to make the
/// "terminal was cleared" assertion non-vacuous.
fn redraw_buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let mut s = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            s.push_str(buf[(x, y)].symbol());
        }
    }
    s
}

/// `needs_clear = true` must (a) clear the terminal's diff buffer so the next
/// frame repaints every cell, and (b) authorise the render by raising
/// `render_pending` and clearing `skip_next_render`.
#[test]
fn apply_force_redraw_clears_terminal_and_sets_flags_when_needs_clear() {
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Paragraph;

    let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 4)).unwrap();
    // Paint a distinctive marker so the clear assertion below is not vacuous.
    terminal
        .draw(|f| f.render_widget(Paragraph::new("markerword"), f.area()))
        .unwrap();
    assert!(
        redraw_buffer_text(terminal.backend().buffer()).contains("markerword"),
        "precondition: the marker must be painted before clear"
    );

    let mut render_pending = false;
    let mut skip_next_render = true;
    apply_force_redraw(
        true,
        &mut terminal,
        &mut render_pending,
        &mut skip_next_render,
    );

    assert!(render_pending, "render_pending must be raised");
    assert!(!skip_next_render, "skip_next_render must be cleared");
    assert!(
        !redraw_buffer_text(terminal.backend().buffer()).contains("markerword"),
        "terminal.clear() must wipe the painted marker so the next frame repaints every cell"
    );
}

/// `needs_clear = false` is a strict no-op: neither flag changes and the
/// terminal is left untouched.
#[test]
fn apply_force_redraw_is_a_noop_when_needs_clear_false() {
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Paragraph;

    let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 4)).unwrap();
    terminal
        .draw(|f| f.render_widget(Paragraph::new("markerword"), f.area()))
        .unwrap();

    let mut render_pending = false;
    let mut skip_next_render = true;
    apply_force_redraw(
        false,
        &mut terminal,
        &mut render_pending,
        &mut skip_next_render,
    );

    assert!(!render_pending, "render_pending must stay untouched");
    assert!(skip_next_render, "skip_next_render must stay untouched");
    assert!(
        redraw_buffer_text(terminal.backend().buffer()).contains("markerword"),
        "needs_clear=false must not clear the terminal"
    );
}

mod skill_apply;
