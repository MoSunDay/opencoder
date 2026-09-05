//! Best-effort mirror of ts-owned sessions into the central ts registry.
//!
//! `opencoder ts` sessions live in the cli registry (`<data_root>/ts.db`); the
//! TUI itself only persists to its per-workdir store. This wrapper sits between
//! the TUI and that store and mirrors the durable index columns (title,
//! preview) the cli needs for `ts -l`/`ts -r`. Plain `tui`/`run` sessions are
//! untouched: a session is treated as ts-owned when its first persisted row
//! carries `model: None` (the `ts_origin` producer contract).
//!
//! All mirror writes are best-effort — failures only log a warning and never
//! propagate into the session flow (same semantics as `SessionState::persist`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{Message, Role};
use opencoder_store::{
    Delivery, ImportReport, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord, TsRecord, TsRegistry,
};
use tokio::sync::Mutex;

const PREVIEW_MAX_CHARS: usize = 80;

/// `Store` wrapper that mirrors ts sessions into the global registry.
pub(crate) struct TsMirrorStore {
    inner: Arc<dyn Store>,
    registry: TsRegistry,
    /// Workdir the TUI was launched in; the registry row's durable workdir.
    workdir: PathBuf,
    /// Sessions created through this instance (fast path: no registry query
    /// on every message append of plain sessions).
    known: Mutex<HashSet<String>>,
}

/// Wrap `inner` with the mirror only when a registry database already exists —
/// a pure `tui`/`run` with no `ts` usage ever sees the registry (zero impact).
pub(crate) async fn maybe_wrap(inner: Arc<dyn Store>, workdir: &Path) -> Arc<dyn Store> {
    maybe_wrap_at(inner, workdir, opencoder_core::data_root().join("ts.db")).await
}

/// Testable seam: wrap only when `ts_db` exists and opens cleanly.
pub(crate) async fn maybe_wrap_at(
    inner: Arc<dyn Store>,
    workdir: &Path,
    ts_db: PathBuf,
) -> Arc<dyn Store> {
    if !ts_db.is_file() {
        return inner;
    }
    match TsRegistry::open(&ts_db).await {
        Ok(registry) => Arc::new(TsMirrorStore {
            inner,
            registry,
            workdir: workdir.to_path_buf(),
            known: Mutex::new(HashSet::new()),
        }),
        Err(error) => {
            tracing::warn!(path = %ts_db.display(), %error, "ts mirror: cannot open registry, continuing without mirror");
            inner
        }
    }
}

impl TsMirrorStore {
    /// Intercept: a first persisted row with `model: None` is the durable
    /// `ts_origin` marker — register it (idempotent upsert) and remember it.
    async fn note_ts_session(&self, meta: &SessionMeta) {
        self.known.lock().await.insert(meta.id.clone());
        let canonical = tokio::fs::canonicalize(&self.workdir)
            .await
            .unwrap_or_else(|_| self.workdir.clone());
        let now = opencoder_core::message::now_ms();
        let record = TsRecord {
            id: meta.id.clone(),
            workdir: Some(canonical.clone()),
            store_dir: Some(opencoder_core::data_dir_for(&canonical)),
            created_at: if meta.created_at > 0 {
                meta.created_at
            } else {
                now
            },
            updated_at: if meta.updated_at > 0 {
                meta.updated_at
            } else {
                now
            },
            title: meta.title.clone(),
            preview: String::new(),
        };
        if let Err(error) = self.registry.upsert(&record).await {
            tracing::warn!(session_id = %meta.id, %error, "ts mirror: cannot register session");
        }
    }

    /// Intercept: the first non-synthetic user message of a known ts session
    /// fills the registry preview (write-once, aligned with the store's
    /// `extract_preview` semantics).
    async fn mirror_preview(&self, session_id: &str, msg: &Message) {
        if msg.role != Role::User || msg.synthetic {
            return;
        }
        if !self.known.lock().await.contains(session_id) {
            return;
        }
        let Ok(Some(mut record)) = self.registry.get(session_id).await else {
            return;
        };
        if !record.preview.is_empty() {
            return;
        }
        let text = msg.text();
        if text.trim().is_empty() {
            return;
        }
        record.preview = text.chars().take(PREVIEW_MAX_CHARS).collect();
        record.updated_at = opencoder_core::message::now_ms();
        if let Err(error) = self.registry.upsert(&record).await {
            tracing::warn!(session_id, %error, "ts mirror: cannot write preview");
        }
    }

    /// Intercept: a title patch (resume-time LLM title) mirrors only when a
    /// registry row already exists — plain sessions never gain one here.
    async fn mirror_title(&self, id: &str, title: &str) {
        let Ok(Some(mut record)) = self.registry.get(id).await else {
            return;
        };
        record.title = Some(title.to_string());
        record.updated_at = opencoder_core::message::now_ms();
        if let Err(error) = self.registry.upsert(&record).await {
            tracing::warn!(session_id = id, %error, "ts mirror: cannot write title");
        }
    }
}

#[async_trait]
impl Store for TsMirrorStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        self.inner.create_session(meta).await?;
        if meta.model.is_none() {
            self.note_ts_session(meta).await;
        }
        Ok(())
    }

    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }

    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(filter).await
    }

    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, patch).await?;
        if let Some(title) = &patch.title {
            self.mirror_title(id, title).await;
        }
        Ok(())
    }

    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await?;
        self.known.lock().await.remove(id);
        if let Err(error) = self.registry.delete(id).await {
            tracing::warn!(session_id = id, %error, "ts mirror: cannot unregister session");
        }
        Ok(())
    }

    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64> {
        let removed = self.inner.clear_other_sessions(keep_session_id).await?;
        self.known.lock().await.retain(|id| id == keep_session_id);
        // The inner store prunes rows via SQL, so per-row delete intercepts
        // never fire: prune the registry index here instead.
        let Ok(records) = self.registry.list().await else {
            return Ok(removed);
        };
        for record in records {
            if record.id != keep_session_id {
                if let Err(error) = self.registry.delete(&record.id).await {
                    tracing::warn!(session_id = %record.id, %error, "ts mirror: cannot unregister cleared session");
                }
            }
        }
        Ok(removed)
    }

    async fn append_message(&self, session_id: &str, msg: &Message) -> Result<i64> {
        let seq = self.inner.append_message(session_id, msg).await?;
        self.mirror_preview(session_id, msg).await;
        Ok(seq)
    }

    async fn append_messages(&self, session_id: &str, msgs: &[Message]) -> Result<Vec<i64>> {
        let seqs = self.inner.append_messages(session_id, msgs).await?;
        for msg in msgs {
            self.mirror_preview(session_id, msg).await;
        }
        Ok(seqs)
    }

    async fn load_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(session_id).await
    }

    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        self.inner.last_message_seq(session_id).await
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.inner.admit_input(input).await
    }

    async fn pending_inputs(
        &self,
        session_id: &str,
        delivery: Delivery,
    ) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(session_id, delivery).await
    }

    async fn promote_inputs(
        &self,
        session_id: &str,
        up_to_admitted_seq: i64,
        delivery: Delivery,
    ) -> Result<Vec<i64>> {
        self.inner
            .promote_inputs(session_id, up_to_admitted_seq, delivery)
            .await
    }

    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(session_id).await
    }

    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(session_id).await
    }

    async fn delete_input(&self, input_id: i64) -> Result<()> {
        self.inner.delete_input(input_id).await
    }

    async fn swap_input_order(&self, session_id: &str, seq_a: i64, seq_b: i64) -> Result<()> {
        self.inner.swap_input_order(session_id, seq_a, seq_b).await
    }

    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(events).await
    }

    async fn events_after(
        &self,
        session_id: &str,
        after_seq: i64,
    ) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(session_id, after_seq).await
    }

    async fn last_event_seq(&self, session_id: &str) -> Result<i64> {
        self.inner.last_event_seq(session_id).await
    }

    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(record).await
    }

    async fn complete_subagent_task(&self, task_id: &str, result: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(task_id, result, ok).await
    }

    async fn list_subagent_tasks(
        &self,
        parent_session_id: &str,
    ) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(parent_session_id).await
    }

    async fn get_subagent_task(&self, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(task_id).await
    }

    async fn cancel_subagent_task(&self, task_id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(task_id).await
    }

    async fn import_messages(&self, session_id: &str, msgs: &[Message]) -> Result<ImportReport> {
        self.inner.import_messages(session_id, msgs).await
    }
}

#[cfg(test)]
#[path = "ts_mirror_tests.rs"]
mod tests;
