//! Integration test for the COMPLETE plan→act event flow through `ChatView`,
//! simulating exactly what `fold_ui_events` does when a `SwitchAndStart` ("act")
//! fires.
//!
//! In the live worker (`SwitchAndStart`), three `SessionEvent`s are emitted in
//! order:
//!   1. `AgentSwitch("act")`
//!   2. `TranscriptReset(handoff_msgs)`  — the UI handles this by REPLACING the
//!      whole `ChatView` with a fresh one built from `replay_into_chat`.
//!   3. `PlanHandoff(plan_display)`
//!
//! `fold_ui_events` (app_loop.rs) maps those onto a single `ChatView`:
//!   - `AgentSwitch` and `PlanHandoff` go through `ChatView::apply`.
//!   - `TranscriptReset` goes through `*chat = replay_into_chat(...)` (a full
//!     replacement, NOT `apply`).
//!
//! The dedup logic in `ChatView::apply` for `PlanHandoff` guarantees we never
//! stack a second plan card on top of one already rendered — and
//! `replay_into_chat` itself renders the plan card from the persisted
//! `handoff_plan` session metadata. This test pins down that, regardless of
//! ordering, the view ends up with EXACTLY ONE `ChatBlock::Plan` block.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_session::plan_handoff::handoff_message;
use opencoder_session::SessionEvent;
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};
use opencoder_tui::chat::{ChatBlock, ChatView};
use opencoder_tui::session_ui::replay_into_chat;

/// The finalized plan produced by the plan agent — carried verbatim through the
/// handoff as the display text (what `plan_handoff::handoff` returns and what
/// `PlanHandoff(plan)` + `handoff_plan` meta both carry).
const PLAN_TEXT: &str = "## Plan\n1. step one\n2. step two";

/// Count how many `ChatBlock::Plan` blocks a view currently holds.
fn plan_block_count(chat: &ChatView) -> usize {
    chat.blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Plan { .. }))
        .count()
}

/// Build the plan agent's final assistant message (the source of the plan).
fn assistant_with_plan() -> Message {
    let mut m = Message::assistant("a1");
    m.blocks.push(ContentBlock::text(PLAN_TEXT));
    m
}

/// Set up an in-memory store with one session whose plan-mode transcript and
/// persisted handoff boundary mirror a real plan→act switch:
///   - session created (no handoff metadata yet, exactly like live creation),
///   - 2 plan-mode messages appended (user prompt + plan agent response),
///   - `handoff_seq` + `handoff_plan` persisted via `update_session` (mirrors
///     `worker.rs::SwitchAndStart` calling `store.update_session` after the
///     in-memory handoff mutation).
async fn setup_session(id: &str) -> Arc<dyn Store> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());

    // Create the session with no handoff metadata (realistic — the boundary is
    // only known once the user actually switches to act).
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();

    // The plan-mode transcript that gets trimmed on handoff.
    store
        .append_message(id, &Message::user("u1", "plan something"))
        .await
        .unwrap();
    store
        .append_message(id, &assistant_with_plan())
        .await
        .unwrap();

    // Persist the handoff boundary via update_session — exactly what the worker
    // does on SwitchAndStart so resume reconstructs the focused transcript.
    store
        .update_session(
            id,
            &SessionPatch {
                handoff_seq: Some(2), // 2 plan-mode messages to trim
                handoff_plan: Some(PLAN_TEXT.to_string()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    store
}

/// Forward order: `AgentSwitch("act")` → `TranscriptReset` replay replacement
/// → `PlanHandoff`. This is the real `SwitchAndStart` event sequence.
#[tokio::test]
async fn forward_order_agent_switch_reset_then_handoff_yields_one_plan_block() {
    let session_id = "plan-card-forward";
    let store = setup_session(session_id).await;

    // The synthetic handoff message the worker builds and feeds through
    // TranscriptReset. It's skipped by replay_one (synthetic), so the ONLY
    // block produced by replay_into_chat is the plan card from `handoff_plan`.
    let handoff_msg = handoff_message(PLAN_TEXT);

    // Start in plan mode, then apply AgentSwitch("act") — mirrors the first
    // event emitted by SwitchAndStart.
    let mut chat = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };
    chat.apply(&SessionEvent::AgentSwitch("act".into()));
    assert_eq!(
        plan_block_count(&chat),
        0,
        "AgentSwitch alone must not create a Plan block"
    );
    assert_eq!(chat.agent, "act", "AgentSwitch must flip the agent to act");

    // TranscriptReset handling: REPLACE the ChatView with a fresh replay
    // (this is what fold_ui_events does for TranscriptReset — NOT apply).
    chat = replay_into_chat("act", std::slice::from_ref(&handoff_msg), &store, session_id).await;
    assert_eq!(
        plan_block_count(&chat),
        1,
        "replay_into_chat must render exactly one Plan block from handoff_plan"
    );

    // Now PlanHandoff fires on the SAME (replayed) ChatView.
    chat.apply(&SessionEvent::PlanHandoff(PLAN_TEXT.into()));
    assert_eq!(
        plan_block_count(&chat),
        1,
        "PlanHandoff after a replayed plan card must NOT stack a second one \
         (dedup), expected exactly one Plan block"
    );
}

/// Reverse order: `PlanHandoff` first, then a `TranscriptReset`-style
/// `replay_into_chat` replacement. Asserts the result is STILL exactly one
/// Plan block — the replay rebuilds the view from persisted metadata, wiping
/// the manually-applied card and re-rendering a single one.
#[tokio::test]
async fn reverse_order_handoff_then_reset_replay_yields_one_plan_block() {
    let session_id = "plan-card-reverse";
    let store = setup_session(session_id).await;

    let handoff_msg = handoff_message(PLAN_TEXT);

    // PlanHandoff FIRST, on a fresh view (no prior card).
    let mut chat = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };
    chat.apply(&SessionEvent::PlanHandoff(PLAN_TEXT.into()));
    assert_eq!(
        plan_block_count(&chat),
        1,
        "PlanHandoff on a fresh view must create exactly one Plan block"
    );

    // TranscriptReset handling: REPLACE the ChatView with a fresh replay.
    chat = replay_into_chat("act", &[handoff_msg], &store, session_id).await;
    assert_eq!(
        plan_block_count(&chat),
        1,
        "reverse order: replay_into_chat replacement must leave exactly one \
         Plan block (rebuilt from handoff_plan), got {}",
        plan_block_count(&chat)
    );
}
