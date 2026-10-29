//! `append_subtask_result` against an in-memory store: terminal entries
//! append through the queue mechanism, non-terminal ones only flash.

use super::*;
use opencoder_core::harness::{HarnessRuntime, SubtaskRef};
use opencoder_store::{SessionMeta, TursoStore};

fn entry(id: &str, status: &str) -> SubtaskRef {
    SubtaskRef {
        execution_id: id.into(),
        kind: "agent".into(),
        capability_id: "ops_x".into(),
        question: "check the logs".into(),
        status: status.into(),
        result_summary: Some("all clear".into()),
        tokens: 42,
        created_at: 0,
    }
}

async fn store_with_registry(sid: &str, refs: Vec<SubtaskRef>) -> Arc<dyn Store> {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let runtime = HarnessRuntime {
        subtasks: refs,
        ..Default::default()
    };
    store.set_harness_runtime(sid, &runtime).await.unwrap();
    store
}

/// A live parent session with a terminal subtask: the append lands as an
/// optimistic queue row in the LIVE panel and no switch is requested.
#[tokio::test]
async fn terminal_entry_from_same_session_appends_to_queue_items() {
    let store = store_with_registry("parent", vec![entry("agent-1", "done")]).await;
    let (admit_tx, _rx) = mpsc::channel(8);
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let mut chat = ChatView::default();
    let mut queue_items = Vec::new();
    let mut pending_images = Vec::new();
    let mut flash = None;
    let pick = append_subtask_result(
        "agent-1",
        &store,
        "parent",
        &mut chat,
        Some("parent"),
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut flash,
        0,
    )
    .await;
    assert!(pick.is_none(), "parent is live: no switch requested");
    assert_eq!(queue_items.len(), 1, "queue panel carries the append row");
    assert!(
        queue_items[0].0 < 0,
        "optimistic temp seq until the actor lands"
    );
    assert!(
        queue_items[0].1.contains("[subtask agent-1"),
        "row renders the append text, got: {}",
        queue_items[0].1
    );
    assert_eq!(
        flash.as_ref().map(|(t, _)| t.as_str()),
        Some("subtask 结果已追加到主任务"),
        "success flash"
    );
}

/// A still-running subtask must not append: only the guard flash fires.
#[tokio::test]
async fn non_terminal_entry_flashes_and_does_not_append() {
    let store = store_with_registry("parent", vec![entry("agent-1", "running")]).await;
    let (admit_tx, _rx) = mpsc::channel(8);
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let mut chat = ChatView::default();
    let mut queue_items = Vec::new();
    let mut pending_images = Vec::new();
    let mut flash = None;
    let pick = append_subtask_result(
        "agent-1",
        &store,
        "child",
        &mut chat,
        Some("parent"),
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut flash,
        3,
    )
    .await;
    assert!(pick.is_none());
    assert!(
        queue_items.is_empty(),
        "nothing admitted for a running subtask"
    );
    assert_eq!(
        flash.as_ref().map(|(t, _)| t.as_str()),
        Some("subtask 尚未完成，完成后再追加"),
        "guard flash"
    );
}

/// Appending from the CHILD view: the row goes to a scratch panel (the child
/// queue panel stays clean) and the parent pick is returned for the switch.
#[tokio::test]
async fn terminal_entry_from_child_session_returns_parent_pick() {
    let store = store_with_registry("parent", vec![entry("agent-1", "done")]).await;
    let (admit_tx, _rx) = mpsc::channel(8);
    let mut admit_st = crate::queue_admitter::AdmitUiState::default();
    let mut chat = ChatView::default();
    let mut queue_items = Vec::new();
    let mut pending_images = Vec::new();
    let mut flash = None;
    let pick = append_subtask_result(
        "agent-1",
        &store,
        "child",
        &mut chat,
        Some("parent"),
        &admit_tx,
        &mut admit_st,
        &mut queue_items,
        &mut pending_images,
        &mut flash,
        7,
    )
    .await;
    assert!(matches!(
        pick,
        Some(crate::task::TaskPick::Resume(ref id)) if id == "parent"
    ));
    assert!(
        queue_items.is_empty(),
        "child panel never shows parent rows"
    );
    assert_eq!(
        flash.as_ref().map(|(t, _)| t.as_str()),
        Some("subtask 结果已追加，正在返回主任务")
    );
}
