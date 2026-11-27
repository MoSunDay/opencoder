//! Lazy parent-session row creation for one-shot subtask dispatch.
//!
//! Opening the TUI persists nothing; a session row (and its harness runtime
//! row) is created only when the user actually submits the first input. A
//! normal prompt rides the session crate's lazy `persist()`; an
//! `@capability` dispatch creates the PARENT rows here, at dispatch time.

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_store::{SessionModelOverrides, Store};
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
        .create_session_with_model_overrides(
            &opencoder_store::SessionMeta {
                id: parent_session.to_string(),
                agent: Some(agent.to_string()),
                model: Some(model),
                created_at: now,
                updated_at: now,
                ..Default::default()
            },
            Some(&SessionModelOverrides::default()),
        )
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
        let store: Arc<dyn Store> =
            Arc::new(opencoder_store::TursoStore::open_memory().await.unwrap());
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

    #[tokio::test]
    async fn dispatch_persists_empty_mode_overrides_without_rewriting_legacy_rows() {
        let store = Arc::new(opencoder_store::TursoStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "legacy-parent".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let dyn_store: Arc<dyn Store> = store.clone();

        ensure_parent_runtime(&dyn_store, "mode-aware-parent", "act", "m".into())
            .await
            .unwrap();

        let conn = store.conn().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT id, model_overrides_json FROM sessions \
                 WHERE id IN ('legacy-parent', 'mode-aware-parent') ORDER BY id",
                (),
            )
            .await
            .unwrap();
        let legacy = rows.next().await.unwrap().unwrap();
        assert_eq!(legacy.get::<String>(0).unwrap(), "legacy-parent");
        assert_eq!(legacy.get::<Option<String>>(1).unwrap(), None);
        let created = rows.next().await.unwrap().unwrap();
        assert_eq!(created.get::<String>(0).unwrap(), "mode-aware-parent");
        assert_eq!(
            created.get::<Option<String>>(1).unwrap().as_deref(),
            Some("{}")
        );
        assert!(rows.next().await.unwrap().is_none());
    }

    /// An already-persisted parent runtime is returned untouched — the lazy
    /// create never clobbers registered subtasks or a codex/envs runtime.
    #[tokio::test]
    async fn existing_parent_runtime_is_returned_untouched() {
        let store: Arc<dyn Store> =
            Arc::new(opencoder_store::TursoStore::open_memory().await.unwrap());
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
        store
            .set_harness_runtime("parent-2", &seeded)
            .await
            .unwrap();

        let runtime = ensure_parent_runtime(&store, "parent-2", "act", "m".into())
            .await
            .unwrap();

        assert_eq!(runtime.subtasks.len(), 1);
        assert_eq!(runtime.subtasks[0].execution_id, "exec-1");
    }
}
