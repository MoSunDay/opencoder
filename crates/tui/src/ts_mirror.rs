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
//!
//! Writer discipline: the TUI never holds `ts.db` writable. It keeps a
//! read-only registry handle for its whole life and opens a short-lived
//! writer for the duration of one mirror mutation, dropped once it completes.
//! Every process may write the registry: a mutation running beside an
//! `opencoder ts` command or another TUI's mutation waits inside the
//! engine's transaction retry rather than being refused or deferred, and an
//! idle TUI holds no writer at all.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use opencoder_core::{Message, Role};
use opencoder_store::{
    Delivery, ImportReport, InputAdmission, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionModelOverrides, SessionPatch, Store, SubagentTaskRecord,
    TsRecord, TsRegistry,
};
use tokio::sync::Mutex;

const PREVIEW_MAX_CHARS: usize = 80;

/// `Store` wrapper that mirrors ts sessions into the global registry.
pub(crate) struct TsMirrorStore {
    inner: Arc<dyn Store>,
    /// Read-only view of the registry, held for the TUI's whole life. The
    /// TUI never holds `ts.db` writable, so an idle TUI blocks nobody.
    registry: TsRegistry,
    /// Registry file: every mutation opens a short-lived writer on it for
    /// that mutation only, dropped again once it completes.
    ts_db: PathBuf,
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

/// Testable seam: wrap only when `ts_db` exists and opens read-only. A reader
/// takes no writer, so a ts command (or another mirror) writing the file at
/// startup does not disable the mirror.
pub(crate) async fn maybe_wrap_at(
    inner: Arc<dyn Store>,
    workdir: &Path,
    ts_db: PathBuf,
) -> Arc<dyn Store> {
    if !ts_db.is_file() {
        return inner;
    }
    match TsRegistry::open_reader(&ts_db).await {
        Ok(registry) => Arc::new(TsMirrorStore::new(inner, registry, ts_db, workdir)),
        Err(error) => {
            tracing::warn!(path = %ts_db.display(), %error, "ts mirror: cannot open registry read-only, continuing without mirror");
            inner
        }
    }
}

impl TsMirrorStore {
    fn new(inner: Arc<dyn Store>, registry: TsRegistry, ts_db: PathBuf, workdir: &Path) -> Self {
        Self {
            inner,
            registry,
            ts_db,
            workdir: workdir.to_path_buf(),
            known: Mutex::new(HashSet::new()),
        }
    }

    /// One mirror mutation against a writer opened for this call only and
    /// dropped once it completes. A concurrent writer (a ts command, another
    /// TUI) makes the mutation wait inside the transaction retry; it is never
    /// refused, and the TUI holds no writer between mutations.
    async fn mutate<T, F, Fut>(&self, mutation: F) -> Result<T>
    where
        F: FnOnce(TsRegistry) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let writer = TsRegistry::open(&self.ts_db)
            .await
            .with_context(|| format!("open ts registry {} for writing", self.ts_db.display()))?;
        mutation(writer).await
    }

    /// Current registry row through the life-long read-only handle.
    async fn read(&self, id: &str) -> Option<TsRecord> {
        self.registry.get(id).await.ok().flatten()
    }

    /// Mirror updates are best-effort: a failure is reported as a skipped
    /// update (never retried, never surfaced into the session flow).
    fn skipped(what: &str, error: &anyhow::Error) {
        tracing::warn!(%error, "ts mirror: {what} skipped (best-effort, not retried)");
    }

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
        if let Err(error) = self
            .mutate(|writer| async move { writer.upsert(&record).await })
            .await
        {
            Self::skipped("register session", &error);
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
        let Some(mut record) = self.read(session_id).await else {
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
        if let Err(error) = self
            .mutate(|writer| async move { writer.upsert(&record).await })
            .await
        {
            Self::skipped("write preview", &error);
        }
    }

    /// Intercept: a title patch (resume-time LLM title) mirrors only when a
    /// registry row already exists — plain sessions never gain one here.
    async fn mirror_title(&self, id: &str, title: &str) {
        let Some(mut record) = self.read(id).await else {
            return;
        };
        record.title = Some(title.to_string());
        record.updated_at = opencoder_core::message::now_ms();
        if let Err(error) = self
            .mutate(|writer| async move { writer.upsert(&record).await })
            .await
        {
            Self::skipped("write title", &error);
        }
    }
}

#[async_trait]
impl Store for TsMirrorStore {
    async fn set_message_usage(
        &self,
        session_id: &str,
        message_id: &str,
        usage: &opencoder_core::MessageUsage,
    ) -> Result<()> {
        self.inner
            .set_message_usage(session_id, message_id, usage)
            .await
    }

    async fn harness_runtime(
        &self,
        id: &str,
    ) -> Result<Option<opencoder_core::harness::HarnessRuntime>> {
        self.inner.harness_runtime(id).await
    }
    async fn set_harness_runtime(
        &self,
        id: &str,
        runtime: &opencoder_core::harness::HarnessRuntime,
    ) -> Result<()> {
        self.inner.set_harness_runtime(id, runtime).await
    }

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

    async fn create_session_with_model_overrides(
        &self,
        meta: &SessionMeta,
        overrides: Option<&SessionModelOverrides>,
    ) -> Result<()> {
        self.inner
            .create_session_with_model_overrides(meta, overrides)
            .await?;
        // Match create_session's mirror hook without routing through it and
        // accidentally replacing the caller's NULL/{}/explicit metadata.
        if meta.model.is_none() {
            self.note_ts_session(meta).await;
        }
        Ok(())
    }

    async fn get_session_model_overrides(&self, id: &str) -> Result<Option<SessionModelOverrides>> {
        self.inner.get_session_model_overrides(id).await
    }

    async fn set_session_model_overrides(
        &self,
        id: &str,
        overrides: Option<&SessionModelOverrides>,
    ) -> Result<()> {
        self.inner.set_session_model_overrides(id, overrides).await
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
        let id = id.to_owned();
        if let Err(error) = self
            .mutate(|writer| async move { writer.delete(&id).await })
            .await
        {
            Self::skipped("unregister session", &error);
        }
        Ok(())
    }

    async fn clear_other_sessions(&self, keep_session_id: &str) -> Result<u64> {
        let removed = self.inner.clear_other_sessions(keep_session_id).await?;
        self.known.lock().await.retain(|id| id == keep_session_id);
        // The inner store prunes rows via SQL, so per-row delete intercepts
        // never fire: prune the registry index here instead, in one mutation.
        let Ok(records) = self.registry.list().await else {
            return Ok(removed);
        };
        let stale: Vec<String> = records
            .into_iter()
            .map(|record| record.id)
            .filter(|id| id != keep_session_id)
            .collect();
        if stale.is_empty() {
            return Ok(removed);
        }
        if let Err(error) = self
            .mutate(|writer| async move {
                for id in &stale {
                    writer.delete(id).await?;
                }
                Ok(())
            })
            .await
        {
            Self::skipped("unregister cleared sessions", &error);
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

    /// Delegate to the inner store instead of inheriting the trait default
    /// (full `load_messages` + in-memory drain): Resume on a compacted
    /// session relies on the native OFFSET query to skip the soft-deleted
    /// head without deserializing it, so a damaged head row must not fail a
    /// tail-only load. Mirroring is unaffected — this is a read path.
    async fn load_messages_after(&self, session_id: &str, skip_count: i64) -> Result<Vec<Message>> {
        self.inner.load_messages_after(session_id, skip_count).await
    }

    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        self.inner.last_message_seq(session_id).await
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.inner.admit_input(input).await
    }

    async fn admit_input_once(&self, input: &SessionInput) -> Result<InputAdmission> {
        self.inner.admit_input_once(input).await
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

    // Input lifecycle bookkeeping. The trait defaults are no-op stubs for test
    // fakes; a production wrapper inheriting them silently drops the runner's
    // records in the underlying store. A consumed input would stay
    // promoted-but-unrecorded, so a later `session export` — which reads that
    // store through its own handle — carries it as an orphan and the import
    // re-delivers it; an error-recovery unpromote or a run-entry orphan
    // recovery would be lost the same way. Every lifecycle write and read is
    // delegated; none of them touches the ts registry.
    async fn unpromote_inputs(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        self.inner.unpromote_inputs(session_id, seqs).await
    }

    async fn mark_inputs_recorded(&self, session_id: &str, seqs: &[i64]) -> Result<()> {
        self.inner.mark_inputs_recorded(session_id, seqs).await
    }

    async fn recover_orphan_inputs(&self, session_id: &str) -> Result<u64> {
        self.inner.recover_orphan_inputs(session_id).await
    }

    async fn orphan_inputs(&self, session_id: &str) -> Result<Vec<SessionInput>> {
        self.inner.orphan_inputs(session_id).await
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
mod model_metadata_tests {
    use super::*;
    use opencoder_store::TursoStore;

    async fn fixture() -> (TsMirrorStore, tempfile::TempDir) {
        let inner: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path().join("project");
        tokio::fs::create_dir_all(&workdir).await.unwrap();
        let ts_db = tmp.path().join("ts.db");
        TsRegistry::open(&ts_db).await.unwrap();
        let registry = TsRegistry::open_reader(&ts_db).await.unwrap();
        (TsMirrorStore::new(inner, registry, ts_db, &workdir), tmp)
    }

    fn ts_meta(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn model_metadata_delegates_all_states_and_keeps_ts_mirroring() {
        let (mirror, _tmp) = fixture().await;
        let empty = SessionModelOverrides::default();
        let explicit = SessionModelOverrides {
            shared: Some("provider-s/shared-model".into()),
            act: Some("provider-a/act-model".into()),
            plan: Some("provider-b/plan-model".into()),
        };

        for (id, overrides) in [
            ("legacy", None),
            ("mode-aware", Some(&empty)),
            ("explicit", Some(&explicit)),
        ] {
            mirror
                .create_session_with_model_overrides(&ts_meta(id), overrides)
                .await
                .unwrap();
        }

        for (id, expected) in [
            ("legacy", None),
            ("mode-aware", Some(empty.clone())),
            ("explicit", Some(explicit.clone())),
        ] {
            assert_eq!(
                mirror.get_session_model_overrides(id).await.unwrap(),
                expected
            );
            assert!(
                mirror.registry.get(id).await.unwrap().is_some(),
                "metadata-aware creation must retain the ts mirror hook"
            );
        }

        let mut plain = ts_meta("plain");
        plain.model = Some("provider/plain".into());
        mirror
            .create_session_with_model_overrides(&plain, Some(&explicit))
            .await
            .unwrap();
        assert_eq!(
            mirror.get_session_model_overrides("plain").await.unwrap(),
            Some(explicit.clone())
        );
        assert!(
            mirror.registry.get("plain").await.unwrap().is_none(),
            "metadata-aware creation must not register a plain session"
        );

        let edited = SessionModelOverrides {
            shared: Some("provider-s/edited-shared".into()),
            act: Some("provider-c/edited-act".into()),
            plan: None,
        };
        mirror
            .set_session_model_overrides("legacy", Some(&edited))
            .await
            .unwrap();
        assert_eq!(
            mirror.get_session_model_overrides("legacy").await.unwrap(),
            Some(edited)
        );
        mirror
            .set_session_model_overrides("explicit", None)
            .await
            .unwrap();
        assert_eq!(
            mirror
                .get_session_model_overrides("explicit")
                .await
                .unwrap(),
            None
        );
    }
}

#[cfg(test)]
#[path = "ts_mirror_tests.rs"]
mod tests;
