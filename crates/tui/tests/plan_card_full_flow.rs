//! Integration tests for the persisted handoff card through
//! `replay_into_chat` — the legacy-compat surface that must keep working
//! after the plan/act dual-mode removal:
//!   - a plain `handoff_plan` renders exactly one read-only `ChatBlock::Plan`
//!     card on replay (dedup/idempotence across repeated replays);
//!   - the clear-context blank sentinel renders NO card and the raw marker
//!     never reaches the UI;
//!   - a clear-context seed boundary (`<<OPENCODER_CLEAR_SEED>>` prefix) is
//!     stripped to its preserved text and renders like a plan card.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_session::handoff::handoff_message;
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};
use opencoder_tui::chat::{ChatBlock, ChatView};
use opencoder_tui::session_ui::replay_into_chat;

/// The finalized plan carried verbatim through the handoff as the display
/// text (what `handoff_plan` meta persists on an agent switch).
const PLAN_TEXT: &str = "## Plan\n1. step one\n2. step two";

/// Count how many `ChatBlock::Plan` blocks a view currently holds.
fn plan_block_count(chat: &ChatView) -> usize {
    chat.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Plan { .. }))
        .count()
}

fn assistant_with_plan() -> Message {
    let mut m = Message::assistant("a1");
    m.blocks.push(ContentBlock::text("here is the plan"));
    m
}

/// Create a session with a persisted handoff boundary (`handoff_seq` +
/// `handoff_plan`), the way the worker does when submitting the control
/// command that switches agent / clears context.
async fn setup_session(id: &str) -> Arc<dyn Store> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());

    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // The pre-boundary transcript that gets trimmed on the switch.
    store
        .append_message(id, &Message::user("u1", "plan something"))
        .await
        .unwrap();
    store.append_message(id, &assistant_with_plan()).await
        .unwrap();

    // Persist the handoff boundary via update_session — exactly what the
    // worker does so resume reconstructs the focused transcript.
    store
        .update_session(
            id,
            &SessionPatch {
                handoff_seq: Some(2), // 2 pre-boundary messages to trim
                handoff_plan: Some(PLAN_TEXT.to_string()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    store
}

/// Replay after an agent switch renders EXACTLY one plan card from the
/// persisted `handoff_plan`, and replaying again (fresh replacement, the
/// TranscriptReset/resume path) stays at exactly one — no stacking.
#[tokio::test]
async fn replay_renders_exactly_one_plan_card_and_is_idempotent() {
    let session_id = "plan-card-replay";
    let store = setup_session(session_id).await;

    // The synthetic handoff message the worker builds and feeds through
    // TranscriptReset. It's skipped by replay_one (synthetic), so the ONLY
    // block produced by replay_into_chat is the plan card from `handoff_plan`.
    let handoff_msg = handoff_message(PLAN_TEXT);

    let mut chat = replay_into_chat(
        "act",
        std::slice::from_ref(&handoff_msg),
        &store,
        session_id,
        0,
    )
    .await;
    assert_eq!(
        plan_block_count(&chat),
        1,
        "replay_into_chat must render exactly one Plan block from handoff_plan"
    );
    assert_eq!(chat.agent, "act", "replay adopts the session agent");

    // A second full replacement (e.g. /clear or resume again) rebuilds the
    // same single card — the persisted meta is the only card source.
    chat = replay_into_chat("act", &[handoff_msg], &store, session_id, 0).await;
    assert_eq!(
        plan_block_count(&chat),
        1,
        "repeated replay must stay at exactly one Plan block, got {}",
        plan_block_count(&chat)
    );
}

/// The clear-context blank boundary must NEVER render a plan card:
/// `handoff_plan` only holds the internal sentinel so resume can rebuild the
/// fresh-start transcript. `replay_into_chat` skips it — the raw
/// `<<OPENCODER_CLEAR_CONTEXT_MARKER>>` string never reaches the UI.
#[tokio::test]
async fn clear_context_sentinel_renders_no_plan_card() {
    let session_id = "clear-context-no-card";
    let store = setup_session(session_id).await;

    // Overwrite the handoff with the clear-context boundary: this is exactly
    // what `ControlCmd::ClearContext` persists in handoff_plan when nothing
    // is preserved.
    store
        .update_session(
            session_id,
            &SessionPatch {
                handoff_seq: Some(2),
                handoff_plan: Some("<<OPENCODER_CLEAR_CONTEXT_MARKER>>".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let fresh = opencoder_session::control_cmd::fresh_start_message();
    let chat = replay_into_chat("act", &[fresh], &store, session_id, 0).await;

    assert_eq!(
        plan_block_count(&chat),
        0,
        "clear-context sentinel must not render a Plan card, got {}",
        plan_block_count(&chat)
    );
    let raw = format!("{:?}", chat.blocks);
    assert!(
        !raw.contains("<<OPENCODER_CLEAR_CONTEXT_MARKER>>"),
        "sentinel must never be output, got: {raw}"
    );
}

/// A clear-context seed boundary strips the `<<OPENCODER_CLEAR_SEED>>` marker
/// and renders ONLY the preserved reply text (like a plan card: read-only
/// continuity context carried across the clear).
#[tokio::test]
async fn clear_context_seed_renders_preserved_text_only() {
    let session_id = "clear-context-seed";
    let store = setup_session(session_id).await;
    let preserved = "the answer was 42";

    store
        .update_session(
            session_id,
            &SessionPatch {
                handoff_seq: Some(2),
                handoff_plan: Some(format!("<<OPENCODER_CLEAR_SEED>>{preserved}")),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let seed = opencoder_session::control_cmd::seed_message(preserved);
    let chat = replay_into_chat("act", &[seed], &store, session_id, 0).await;

    assert_eq!(
        plan_block_count(&chat),
        1,
        "seed boundary renders its preserved text as the single plan card"
    );
    let raw = format!("{:?}", chat.blocks);
    assert!(
        raw.contains(preserved),
        "preserved text must be visible, got: {raw}"
    );
    assert!(
        !raw.contains("<<OPENCODER_CLEAR_SEED>>"),
        "the raw seed marker must never reach the UI, got: {raw}"
    );
}
