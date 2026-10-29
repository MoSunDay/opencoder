//! Restore-floor lock for one-shot remote subtasks across `/task` switches.
//!
//! Switching away snapshots the live `[tok cost]` (usage replay + subtask
//! folds so far) plus the registry token baseline (`subtask_tokens_seen`).
//! While the session is dormant, a watcher can complete MORE subtasks and
//! grow the registry behind the snapshot's back. Switching back must restore
//! `max(replayed usage, snapshot + registry growth)` — never below what the
//! user saw, never double-counting the folds already inside the snapshot.

use std::sync::Arc;

use opencoder_core::{harness::SubtaskRef, Message};
use opencoder_store::{SessionMeta, Store, TursoStore};
use opencoder_tui::session_ui::{replay_into_chat, subtask_restore_floor, SessionUiState};
use opencoder_tui::subtask_registry::sum_tokens;

const PARENT: &str = "parent-1";

fn entry(id: &str, status: &str, tokens: u64) -> SubtaskRef {
    SubtaskRef {
        execution_id: id.into(),
        kind: "agent".into(),
        capability_id: "ops_x".into(),
        question: format!("question {id}"),
        status: status.into(),
        result_summary: None,
        tokens,
        created_at: 0,
    }
}

fn assistant_with_usage(id: &str, total_tokens: u64) -> Message {
    let mut message = Message::assistant(id);
    message.usage.total_tokens = total_tokens;
    message
}

async fn fresh_store() -> Arc<dyn Store> {
    let store = TursoStore::open_memory().await.unwrap();
    store
        .create_session(&SessionMeta {
            id: PARENT.into(),
            agent: Some("act".into()),
            model: Some("m".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    Arc::new(store)
}

async fn registry_of(store: &Arc<dyn Store>) -> Vec<SubtaskRef> {
    store
        .harness_runtime(PARENT)
        .await
        .unwrap()
        .map(|runtime| runtime.subtasks)
        .unwrap_or_default()
}

async fn save_registry(store: &Arc<dyn Store>, refs: Vec<SubtaskRef>) {
    let mut runtime = store
        .harness_runtime(PARENT)
        .await
        .unwrap()
        .unwrap_or_default();
    runtime.subtasks = refs;
    store.set_harness_runtime(PARENT, &runtime).await.unwrap();
}

async fn transcript(store: &Arc<dyn Store>) -> Vec<Message> {
    store.append_messages(PARENT, &[assistant_with_usage("a1", 50), assistant_with_usage("a2", 50)])
        .await
        .unwrap();
    store.load_messages(PARENT).await.unwrap()
}

#[tokio::test]
async fn switch_away_and_back_never_regresses_below_snapshot_plus_registry_growth() {
    let store = fresh_store().await;
    let messages = transcript(&store).await;

    // Fresh visit: replay folds persisted usage only (2 × 50).
    let mut chat = replay_into_chat("act", &messages, &store, PARENT, 0).await;
    assert_eq!(chat.tokens_total, 100, "two replayed assistant usages");

    // Live path: one subtask completes and its 40 tokens fold in.
    let first_registry = vec![entry("agent-a", "done", 40)];
    save_registry(&store, first_registry.clone()).await;
    chat.tokens_total = chat.tokens_total.saturating_add(40);
    assert_eq!(chat.tokens_total, 140, "live fold mirrors apply_update");

    // Switch away: the snapshot freezes 140 and the Σregistry baseline 40.
    let snapshot = SessionUiState::snapshot(
        false,
        &chat,
        &[],
        0,
        true,
        0,
        0,
        &[],
        &None,
        &None,
        sum_tokens(&first_registry),
    );
    assert_eq!(snapshot.chat.tokens_total, 140);
    assert_eq!(snapshot.subtask_tokens_seen, 40);

    // Dormant growth: a watcher completes a second subtask (90 tokens)
    // while the session is away — only the STORE registry moves.
    let mut grown = first_registry.clone();
    grown.push(entry("agent-b", "done", 90));
    save_registry(&store, grown).await;

    // Switch back: floor = snapshot total + registry growth since baseline.
    let registry_now = registry_of(&store).await;
    let sum_now = sum_tokens(&registry_now);
    assert_eq!(sum_now, 130, "40 already seen + 90 dormant");
    let floor = subtask_restore_floor(
        snapshot.chat.tokens_total,
        snapshot.subtask_tokens_seen,
        sum_now,
    );
    assert_eq!(floor, 230, "140 + (130 - 40): dormant growth only");

    let restored = replay_into_chat("act", &messages, &store, PARENT, floor).await;
    assert!(
        restored.tokens_total >= floor,
        "restore must never regress below the floor, got {}",
        restored.tokens_total
    );
    assert_eq!(
        restored.tokens_total, 230,
        "max(replayed 100, floor 230) — the first 40 lives in the snapshot, no double count"
    );
}

/// A session visited for the FIRST time has no snapshot: the registry total
/// IS the floor, so a watcher's dormant completions still surface.
#[tokio::test]
async fn fresh_first_visit_floor_is_the_whole_registry_sum() {
    let store = fresh_store().await;
    let messages = transcript(&store).await;
    save_registry(
        &store,
        vec![entry("agent-a", "done", 40), entry("agent-b", "done", 90)],
    )
    .await;

    let registry_now = registry_of(&store).await;
    let floor = subtask_restore_floor(0, 0, sum_tokens(&registry_now));
    assert_eq!(floor, 130, "no snapshot: the whole Σregistry applies");

    let chat = replay_into_chat("act", &messages, &store, PARENT, floor).await;
    assert_eq!(chat.tokens_total, 130, "usage 100 loses to the registry floor");
}
