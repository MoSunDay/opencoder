//! Lazy parent-session row creation for one-shot subtask dispatch.
//!
//! Opening the TUI persists nothing; a session row (and its harness runtime
//! row) is created only when the user actually submits the first input. A
//! normal prompt rides the session crate's lazy `persist()`; an
//! `@capability` dispatch creates the PARENT rows here, at dispatch time.

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_store::Store;
use std::sync::Arc;

/// Load the parent session's runtime for a registry write, creating the
/// session row (plus a fresh default runtime) when this dispatch is the
/// session's FIRST input. Opening the TUI persists nothing, so the parent
/// row may legitimately not exist yet; `set_harness_runtime` updates rather
/// than inserts, so the row is created first — the store's `INSERT OR IGNORE`
/// keeps the first prompt's later lazy `create_session` a harmless no-op.
pub(crate) async fn ensure_parent_runtime(
    store: &Arc<dyn Store>,
    parent_session: &str,
    agent: &str,
    model: String,
) -> Result<opencoder_core::harness::HarnessRuntime> {
    if let Some(existing) = store.harness_runtime(parent_session).await? {
        return Ok(existing);
    }
    let now = now_ms();
    store
        .create_session(&opencoder_store::SessionMeta {
            id: parent_session.to_string(),
            agent: Some(agent.to_string()),
            model: Some(model),
            created_at: now,
            updated_at: now,
            ..Default::default()
        })
        .await?;
    let runtime = opencoder_core::harness::fresh_runtime(
        opencoder_core::harness::agent_harness(agent),
        None,
        Default::default(),
        None,
    );
    store.set_harness_runtime(parent_session, &runtime).await?;
    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::harness::SubtaskRef;

    /// A dispatch as the session's FIRST input must create the parent's
    /// session row and a fresh default runtime right then — opening the TUI
    /// alone persists nothing, so the row cannot be assumed to exist.
    #[tokio::test]
    async fn dispatch_as_first_input_creates_parent_row_and_runtime() {
        let store: Arc<dyn Store> = Arc::new(opencoder_store::TursoStore::open_memory().await.unwrap());
        assert!(
            store.get_session("parent-1").await.unwrap().is_none(),
            "precondition: opening the TUI seeded no task record"
        );

        let runtime = ensure_parent_runtime(&store, "parent-1", "act", "m1".into())
            .await
            .unwrap();

        let meta = store.get_session("parent-1").await.unwrap().unwrap();
        assert_eq!(meta.agent.as_deref(), Some("act"));
        assert_eq!(meta.model.as_deref(), Some("m1"));
        assert!(runtime.subtasks.is_empty(), "fresh registry starts empty");
        assert!(
            store.harness_runtime("parent-1").await.unwrap().is_some(),
            "runtime row persisted for later registry writes"
        );
    }

    /// An already-persisted parent runtime is returned untouched — the lazy
    /// create never clobbers registered subtasks or a codex/envs runtime.
    #[tokio::test]
    async fn existing_parent_runtime_is_returned_untouched() {
        let store: Arc<dyn Store> = Arc::new(opencoder_store::TursoStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "parent-2".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let seeded = opencoder_core::harness::HarnessRuntime {
            subtasks: vec![SubtaskRef {
                execution_id: "exec-1".into(),
                kind: "agent".into(),
                capability_id: "cap".into(),
                question: "q".into(),
                status: "done".into(),
                result_summary: Some("ok".into()),
                tokens: 3,
                created_at: 1,
            }],
            ..Default::default()
        };
        store.set_harness_runtime("parent-2", &seeded).await.unwrap();

        let runtime = ensure_parent_runtime(&store, "parent-2", "act", "m".into())
            .await
            .unwrap();

        assert_eq!(runtime.subtasks.len(), 1);
        assert_eq!(runtime.subtasks[0].execution_id, "exec-1");
    }
}
