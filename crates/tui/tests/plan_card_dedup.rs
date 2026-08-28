//! Plan-card dedup on replay: however many sources could produce a card on a
//! single replay (persisted `handoff_plan` meta + the synthetic handoff
//! message + the real transcript rows), the view must hold EXACTLY ONE
//! `ChatBlock::Plan` — and repeated full replays must stay at one.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_session::handoff::handoff_message;
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};
use opencoder_tui::chat::{ChatBlock, ChatView};
use opencoder_tui::session_ui::replay_into_chat;

const PLAN_TEXT: &str = "## Plan\n1. step one\n2. step two";

fn plan_block_count(chat: &ChatView) -> usize {
    chat.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Plan { .. }))
        .count()
}

fn assistant_with_plan() -> Message {
    let mut m = Message::assistant("a2");
    m.blocks.push(ContentBlock::text("here is the plan"));
    m
}

/// Session with a persisted handoff boundary AND its full pre-boundary
/// transcript still in the store — the state right after a legacy switch
/// where nothing was trimmed yet.
async fn setup_session_with_history(id: &str) -> Arc<dyn Store> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_message(id, &Message::user("u1", "plan something"))
        .await
        .unwrap();
    store.append_message(id, &assistant_with_plan()).await.unwrap();
    store
        .update_session(
            id,
            &SessionPatch {
                handoff_seq: Some(2),
                handoff_plan: Some(PLAN_TEXT.to_string()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    store
}

/// The realistic replay payload: the loaded transcript rows PLUS the
/// synthetic handoff message the worker fed through TranscriptReset. Three
/// card-capable inputs (persisted meta, synthetic message, real assistant
/// row) must collapse to exactly one Plan block.
#[tokio::test]
async fn transcript_plus_handoff_message_render_exactly_one_card() {
    let id = "dedup-combined";
    let store = setup_session_with_history(id).await;

    let history = store.load_messages(id).await.unwrap();
    assert_eq!(history.len(), 2, "u1 + the plan assistant row");
    let handoff_msg = handoff_message(PLAN_TEXT);

    let mut payload = history.clone();
    payload.push(handoff_msg);
    let chat = replay_into_chat("act", &payload, &store, id, 0).await;

    assert_eq!(
        plan_block_count(&chat),
        1,
        "meta + synthetic + transcript must collapse to ONE Plan card, got {}",
        plan_block_count(&chat)
    );
    // The transcript content is still rendered around the single card.
    let raw = format!("{:?}", chat.blocks);
    assert!(raw.contains("plan something"), "user row rendered, got: {raw}");
}

/// Replaying the SAME payload again (resume again / TranscriptReset
/// replacement rebuild) must be idempotent: still exactly one card.
#[tokio::test]
async fn repeated_replays_stay_at_exactly_one_card() {
    let id = "dedup-repeat";
    let store = setup_session_with_history(id).await;
    let history = store.load_messages(id).await.unwrap();
    let mut payload = history;
    payload.push(handoff_message(PLAN_TEXT));

    let mut chat = replay_into_chat("act", &payload, &store, id, 0).await;
    assert_eq!(plan_block_count(&chat), 1);

    for round in 2..=3 {
        chat = replay_into_chat("act", &payload, &store, id, 0).await;
        assert_eq!(
            plan_block_count(&chat),
            1,
            "replay round {round} must stay at exactly one Plan card, got {}",
            plan_block_count(&chat)
        );
    }
}

/// A clear-context seed boundary REPLACES the plan card on the next replay:
/// after the fold, the persisted marker is the seed flavour and the view
/// holds exactly one card rendered from the preserved text (the stale plan
/// boundary is gone, not stacked).
#[tokio::test]
async fn seed_boundary_replaces_the_stale_plan_card() {
    let id = "dedup-seed-replace";
    let store = setup_session_with_history(id).await;

    // The fold overwrote the boundary with a seed marker.
    store
        .update_session(
            id,
            &SessionPatch {
                handoff_plan: Some(format!("<<OPENCODER_CLEAR_SEED>>{}", PLAN_TEXT)),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let seed = opencoder_session::seed_message(PLAN_TEXT);
    let chat = replay_into_chat("act", &[seed], &store, id, 0).await;
    assert_eq!(
        plan_block_count(&chat),
        1,
        "the seed renders as the single card, got {}",
        plan_block_count(&chat)
    );
    let raw = format!("{:?}", chat.blocks);
    assert!(
        !raw.contains("<<OPENCODER_CLEAR_SEED>>"),
        "the raw seed marker never reaches the UI, got: {raw}"
    );
    assert!(raw.contains("step one"), "preserved text visible, got: {raw}");
}
