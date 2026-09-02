//! Regression tests for the Plan card rebuild on `TranscriptReset`
//! (`rebuild_after_reset` -> `replay_into_chat`). The rebuild reads the
//! persisted `handoff_plan` boundary: a directive display (or a clear-context
//! seed's preserved text) must re-render as a `ChatBlock::Plan`, while the
//! blank sentinel must never surface. This is the UI half of the re-clear
//! preservation contract pinned in
//! `opencoder-session/tests/clear_context_reclear_preserves.rs`.

use super::replay::rebuild_after_reset;
use crate::chat::{ChatBlock, ChatView};
use opencoder_core::Message;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use std::sync::Arc;

const SEED_MARKER: &str = "<<OPENCODER_CLEAR_SEED>>";
const BLANK_MARKER: &str = "<<OPENCODER_CLEAR_CONTEXT_MARKER>>";

async fn store_with_boundary(session_id: &str, handoff_plan: Option<&str>) -> Arc<dyn Store> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: session_id.into(),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            handoff_seq: Some(2),
            handoff_plan: handoff_plan.map(str::to_string),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
}

fn plan_blocks(chat: &ChatView) -> Vec<&ChatBlock> {
    chat.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Plan { .. }))
        .collect()
}

async fn rebuild_chat(store: &Arc<dyn Store>, session_id: &str, msgs: &[Message]) -> ChatView {
    let mut chat = ChatView::default();
    rebuild_after_reset(&mut chat, msgs, store, session_id).await;
    chat
}

/// A directive boundary (a preserved plan) re-renders the Plan card on reset.
#[tokio::test]
async fn rebuild_keeps_plan_card_for_directive_boundary() {
    let store = store_with_boundary("ui-directive", Some("the plan brief")).await;
    let msgs = vec![Message::user("m1", "Execute it now. the plan brief")];
    let chat = rebuild_chat(&store, "ui-directive", &msgs).await;
    let plans = plan_blocks(&chat);
    assert_eq!(plans.len(), 1, "Plan card must survive the reset rebuild");
    match plans[0] {
        ChatBlock::Plan { raw, .. } => {
            assert_eq!(raw, "the plan brief", "card carries the preserved plan");
        }
        _ => unreachable!(),
    }
}

/// A seed boundary renders the STRIPPED preserved text (raw marker never
/// reaches the UI).
#[tokio::test]
async fn rebuild_renders_seed_boundary_without_raw_marker() {
    let store = store_with_boundary("ui-seed", Some(&format!("{SEED_MARKER}task done"))).await;
    let msgs = vec![Message::user("m1", "prior context task done")];
    let chat = rebuild_chat(&store, "ui-seed", &msgs).await;
    let plans = plan_blocks(&chat);
    assert_eq!(plans.len(), 1);
    match plans[0] {
        ChatBlock::Plan { raw, .. } => {
            assert_eq!(raw, "task done");
            assert!(!raw.contains(SEED_MARKER));
        }
        _ => unreachable!(),
    }
}

/// The blank sentinel (nothing preserved) renders NO Plan card — but that is
/// only reached when the fold really had nothing to preserve; the re-clear
/// fix keeps real boundaries out of this path.
#[tokio::test]
async fn rebuild_skips_plan_card_for_blank_sentinel() {
    let store = store_with_boundary("ui-blank", Some(BLANK_MARKER)).await;
    let msgs = vec![Message::user("m1", "[Context cleared - starting fresh.]")];
    let chat = rebuild_chat(&store, "ui-blank", &msgs).await;
    assert!(
        plan_blocks(&chat).is_empty(),
        "sentinel must never render as a Plan card"
    );
}
