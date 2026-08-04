//! Regression: a quit→resume cycle restores the queue/steer panel mirrors
//! from the store with the display original (`display_text`, may contain the
//! `$skill` token) while the drained prompt stays the token-stripped clean
//! text (LLM contract unchanged).
//!
//! The TUI admit paths now persist `display_text` (raw user text incl. any
//! `$skill` token) alongside the clean `prompt`. On startup (`run_app`) and
//! on `/task` reload (`app_task::switch_session`) the mirrors are rebuilt from
//! `store.pending_inputs` via `restore_pending_mirrors`/`pending_mirror`, so a
//! resumed session shows exactly what the user submitted — and rows admitted
//! before `display_text` existed fall back to `prompt`. Drain still reads only
//! `prompt`; the panel removes consumed rows by the store row seq.
use std::sync::Arc;

use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// Mirror of the app.rs admit shape: prompt is the clean (token-stripped)
/// text, display_text is the raw user input (may carry `$skill`).
fn queued_input(session_id: &str, prompt: &str, display_text: Option<&str>) -> SessionInput {
    SessionInput {
        seq: None,
        id: format!("in-{}", prompt.replace(' ', "-")),
        session_id: session_id.into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: Vec::new(),
        display_text: display_text.map(|d| d.to_string()),
        admitted_seq: 0,
        promoted_seq: None,
    }
}

fn steered_input(session_id: &str, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: format!("steer-{}", prompt.replace(' ', "-")),
        session_id: session_id.into(),
        delivery: Delivery::Steer,
        prompt: prompt.into(),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

#[tokio::test]
async fn resume_restores_display_originals_and_drain_stays_clean() {
    let store = mem_store().await;
    let sid = "resume-demo";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();

    // Phase 1 (pre-quit): user queues `$repo-memory fix the bug` while a
    // skill is active — app.rs admits prompt="fix the bug" +
    // display_text="$repo-memory fix the bug" — and steers a plain follow-up
    // (no distinct display form).
    let q_seq = store
        .admit_input(&queued_input(
            sid,
            "fix the bug",
            Some("$repo-memory fix the bug"),
        ))
        .await
        .unwrap();
    let s_seq = store
        .admit_input(&steered_input(sid, "steer me now"))
        .await
        .unwrap();

    // Phase 2 (resume): run_app / app_task reload rebuild the panel mirrors
    // from pending store rows.
    let mut steer_items: Vec<(i64, String)> = Vec::new();
    let queue_items =
        opencoder_tui::queue_panel::restore_pending_mirrors(&store, sid, &mut steer_items).await;

    assert_eq!(
        queue_items,
        vec![(q_seq, "$repo-memory fix the bug".to_string())],
        "resumed queue panel must show the display original (raw $skill token)"
    );
    assert_eq!(
        steer_items,
        vec![(s_seq, "steer me now".to_string())],
        "resumed steer panel falls back to prompt when display_text is None"
    );

    // Phase 3 (drain): the LLM consumes the clean prompt — never the token —
    // and the panel removes the row by the store row seq (the retain-by-seq
    // contract the event loop relies on).
    let (drained_seq, drained) = store
        .claim_next_queue(sid)
        .await
        .unwrap()
        .expect("queued input must be claimable after resume");
    assert_eq!(drained_seq, q_seq, "drain seq must equal the panel seq");
    assert_eq!(
        drained.prompt, "fix the bug",
        "drained prompt stays token-stripped (LLM contract)"
    );
    assert!(
        !drained.prompt.contains("$"),
        "the $skill token must never reach the LLM"
    );

    let mut panel = queue_items;
    panel.retain(|(s, _)| *s != drained_seq);
    assert!(
        panel.is_empty(),
        "consumed row removed from the panel mirror by seq"
    );

    // The consumed row no longer shows up on a re-restore either.
    let re_restored =
        opencoder_tui::queue_panel::restore_pending_mirrors(&store, sid, &mut steer_items).await;
    assert!(re_restored.is_empty(), "claimed row no longer pending");
}
