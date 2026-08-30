//! One-shot `$skill` lifetime: an activation (inline `$name` token, queue/
//! steer drain, or a pre-set `skill_prompt`) lives ONLY for the run that
//! triggered it.
//!
//! Within the run the skill behaves exactly as before: the body is injected
//! into the persistent transcript once (`skill_context::ensure_full_body_loaded`,
//! guarded by the `[skill loaded]` marker scan) and every LLM round carries
//! the transient `[active skill]` tail reminder. When the run ends — Done,
//! Error, or cancel — [`clear_on_run_end`] wipes the skill from memory
//! (`skill_prompt` + `active_skill_names`) and best-effort from the store
//! (`SessionPatch { clear_skill: true }`), so subsequent runs start
//! skill-less: a second plain submit carries no reminder and unlocks no
//! latent tools.
//!
//! Why clear durably too: `resume` restores `skill_prompt` from the
//! `sessions.skill` column, so leaving the row set would resurrect the skill
//! into every later resumed session — the run-end clear defeats that
//! resurrection. The deliberate exception is a crash MID-run: the row is
//! still set (activation persists at consumption time), so resume keeps the
//! skill and the resumed run continues it — then this same run-end hook
//! clears it once that run completes.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_core::ToolArc;
use opencoder_store::SessionPatch;

use crate::runner::{run_loop, SessionEvent};
use crate::SessionState;

/// Clear the active skill after a run ends: memory (`skill_prompt` +
/// `active_skill_names`) and the store (`clear_skill: true`).
///
/// Idempotent guard: a skill-less session returns immediately WITHOUT a
/// store write, so plain (skill-less) runs never touch `sessions.updated_at`.
///
/// The durable clear is the half that stops later `resume`s from
/// resurrecting the skill into every subsequent run (tail reminder + latent
/// unlocks every turn). It therefore retries through transient store
/// failures (busy/locked WAL under concurrent writers) and, when it still
/// cannot land, emits a visible [`SessionEvent::Status`] instead of
/// swallowing the error — a silently stale row used to re-arm every future
/// run, which is exactly the "reminder on every turn" bug.
pub(crate) async fn clear_on_run_end(
    session: &SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) {
    if session.skill_prompt_cloned().is_none() {
        return;
    }
    session.set_skill(None);
    session.set_active_skill_names(HashSet::new());
    let Some(store) = &session.store else {
        return; // no store attached: no persisted row to resurrect from
    };
    let patch = SessionPatch {
        clear_skill: true,
        updated_at: Some(now_ms()),
        ..Default::default()
    };
    let mut last_err = None;
    for attempt in 0..3u32 {
        match store.update_session(&session.id, &patch).await {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < 2 {
                    tokio::time::sleep(std::time::Duration::from_millis(100u64 << attempt)).await;
                }
            }
        }
    }
    if let Some(e) = last_err {
        on_event(SessionEvent::Status(format!(
            "[skill] run ended but the persisted skill clear failed after retries: {e:#}"
        )));
    }
}

/// [`run_loop`] wrapped with the universal run-end hook: the loop runs
/// unchanged, then the skill is cleared on BOTH outcomes before the original
/// result is returned. Done, Error, and cancel all flow through here (a
/// cancelled loop breaks with `Ok`, an LLM/store failure returns `Err`), so
/// this single wrapper is the one-shot boundary for every caller.
pub(crate) async fn run_loop_one_shot(
    session: &mut SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
    drain_mode: bool,
) -> Result<()> {
    let result = run_loop(session, registry, on_event, drain_mode).await;
    clear_on_run_end(session, on_event).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

    fn make_session(store: Option<Arc<dyn Store>>) -> SessionState {
        let working_dir = std::env::temp_dir().join("opencoder-skill-lifecycle-tests");
        let mut s = SessionState::new(
            "sess-skill-lifecycle",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            working_dir,
        );
        if let Some(store) = store {
            s = s.with_store(store).mark_session_created();
        }
        s
    }

    async fn mem_store_with_row() -> Arc<dyn Store> {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "sess-skill-lifecycle".into(),
                agent: Some("act".into()),
                model: Some("m".into()),
                created_at: 0,
                updated_at: 0,
                ..opencoder_store::SessionMeta::default()
            })
            .await
            .unwrap();
        store
    }

    /// Guard: no active skill → early return, no store write. Proven via
    /// `updated_at`: the clear path stamps `now_ms()`, so an unchanged seed
    /// value means `update_session` never ran.
    #[tokio::test]
    async fn no_skill_is_a_no_op_without_store_write() {
        let store = mem_store_with_row().await;
        let s = make_session(Some(store.clone()));

        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;

        assert!(s.skill_prompt_cloned().is_none(), "still skill-less");
        let meta = store.get_session("sess-skill-lifecycle").await.unwrap();
        assert_eq!(
            meta.map(|m| m.updated_at),
            Some(0),
            "no store write on the skill-less guard path"
        );
    }

    /// Skill present → cleared from memory AND from the store row.
    #[tokio::test]
    async fn active_skill_cleared_in_memory_and_store() {
        let store = mem_store_with_row().await;
        let s = make_session(Some(store.clone()));
        s.set_skill(Some("> Source: /skills/a/SKILL.md\n\nBODY".into()));
        s.set_active_skill_names(["a".into()].into_iter().collect());

        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;

        assert!(s.skill_prompt_cloned().is_none(), "memory skill cleared");
        assert!(
            s.active_skill_names_cloned().is_empty(),
            "active_skill_names cleared"
        );
        let meta = store.get_session("sess-skill-lifecycle").await.unwrap();
        assert!(
            meta.and_then(|m| m.skill).is_none(),
            "store row skill cleared"
        );
    }

    /// Idempotence: a second clear after the first is a pure guard no-op.
    #[tokio::test]
    async fn clear_is_idempotent() {
        let store = mem_store_with_row().await;
        let s = make_session(Some(store));
        s.set_skill(Some("BODY".into()));
        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;
        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;
        assert!(s.skill_prompt_cloned().is_none());
    }

    /// Store-less sessions (TUI in-memory path) still clear in memory.
    #[tokio::test]
    async fn store_less_session_clears_memory_only() {
        let s = make_session(None);
        s.set_skill(Some("BODY".into()));
        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;
        assert!(s.skill_prompt_cloned().is_none());
    }

    /// Store double for the retry contract: `update_session` fails the first
    /// `fail_first` calls, then delegates; every attempt is counted.
    struct RetryProbeStore {
        inner: Arc<LibsqlStore>,
        attempts: AtomicUsize,
        fail_first: usize,
    }

    #[async_trait::async_trait]
    impl Store for RetryProbeStore {
        fn backend_name(&self) -> &'static str {
            "retry-probe"
        }
        async fn create_session(&self, m: &SessionMeta) -> anyhow::Result<()> {
            self.inner.create_session(m).await
        }
        async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionMeta>> {
            self.inner.get_session(id).await
        }
        async fn list_sessions(
            &self,
            f: &opencoder_store::SessionFilter,
        ) -> anyhow::Result<Vec<opencoder_store::SessionListItem>> {
            self.inner.list_sessions(f).await
        }
        async fn update_session(&self, id: &str, patch: &SessionPatch) -> anyhow::Result<()> {
            let n = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if n <= self.fail_first {
                anyhow::bail!("simulated skill-clear failure #{n}");
            }
            self.inner.update_session(id, patch).await
        }
        async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
            self.inner.delete_session(id).await
        }
        async fn clear_other_sessions(&self, keep: &str) -> anyhow::Result<u64> {
            self.inner.clear_other_sessions(keep).await
        }
        async fn append_message(
            &self,
            sid: &str,
            m: &opencoder_core::Message,
        ) -> anyhow::Result<i64> {
            self.inner.append_message(sid, m).await
        }
        async fn append_messages(
            &self,
            sid: &str,
            msgs: &[opencoder_core::Message],
        ) -> anyhow::Result<Vec<i64>> {
            self.inner.append_messages(sid, msgs).await
        }
        async fn load_messages(
            &self,
            sid: &str,
        ) -> anyhow::Result<Vec<opencoder_core::Message>> {
            self.inner.load_messages(sid).await
        }
        async fn last_message_seq(&self, sid: &str) -> anyhow::Result<i64> {
            self.inner.last_message_seq(sid).await
        }
        async fn admit_input(
            &self,
            input: &opencoder_store::SessionInput,
        ) -> anyhow::Result<i64> {
            self.inner.admit_input(input).await
        }
        async fn pending_inputs(
            &self,
            sid: &str,
            d: opencoder_store::Delivery,
        ) -> anyhow::Result<Vec<opencoder_store::SessionInput>> {
            self.inner.pending_inputs(sid, d).await
        }
        async fn promote_inputs(
            &self,
            sid: &str,
            up_to: i64,
            d: opencoder_store::Delivery,
        ) -> anyhow::Result<Vec<i64>> {
            self.inner.promote_inputs(sid, up_to, d).await
        }
        async fn promote_next_queued(&self, sid: &str) -> anyhow::Result<Option<i64>> {
            self.inner.promote_next_queued(sid).await
        }
        async fn claim_next_queue(
            &self,
            sid: &str,
        ) -> anyhow::Result<Option<(i64, opencoder_store::SessionInput)>> {
            self.inner.claim_next_queue(sid).await
        }
        async fn unpromote_inputs(&self, sid: &str, seqs: &[i64]) -> anyhow::Result<()> {
            self.inner.unpromote_inputs(sid, seqs).await
        }
        async fn mark_inputs_recorded(&self, sid: &str, seqs: &[i64]) -> anyhow::Result<()> {
            self.inner.mark_inputs_recorded(sid, seqs).await
        }
        async fn recover_orphan_inputs(&self, sid: &str) -> anyhow::Result<u64> {
            self.inner.recover_orphan_inputs(sid).await
        }
        async fn delete_input(&self, id: i64) -> anyhow::Result<()> {
            self.inner.delete_input(id).await
        }
        async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> anyhow::Result<()> {
            self.inner.swap_input_order(sid, a, b).await
        }
        async fn append_events(
            &self,
            evs: &[opencoder_store::SessionEventRecord],
        ) -> anyhow::Result<Vec<i64>> {
            self.inner.append_events(evs).await
        }
        async fn events_after(
            &self,
            sid: &str,
            after: i64,
        ) -> anyhow::Result<Vec<opencoder_store::SessionEventRecord>> {
            self.inner.events_after(sid, after).await
        }
        async fn last_event_seq(&self, sid: &str) -> anyhow::Result<i64> {
            self.inner.last_event_seq(sid).await
        }
        async fn create_subagent_task(
            &self,
            r: &opencoder_store::SubagentTaskRecord,
        ) -> anyhow::Result<()> {
            self.inner.create_subagent_task(r).await
        }
        async fn complete_subagent_task(
            &self,
            id: &str,
            res: &str,
            ok: bool,
        ) -> anyhow::Result<()> {
            self.inner.complete_subagent_task(id, res, ok).await
        }
        async fn list_subagent_tasks(
            &self,
            sid: &str,
        ) -> anyhow::Result<Vec<opencoder_store::SubagentTaskRecord>> {
            self.inner.list_subagent_tasks(sid).await
        }
        async fn get_subagent_task(
            &self,
            id: &str,
        ) -> anyhow::Result<Option<opencoder_store::SubagentTaskRecord>> {
            self.inner.get_subagent_task(id).await
        }
        async fn cancel_subagent_task(&self, id: &str) -> anyhow::Result<()> {
            self.inner.cancel_subagent_task(id).await
        }
    }

    async fn probe_store(fail_first: usize) -> (Arc<RetryProbeStore>, Arc<dyn Store>) {
        let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
        inner
            .create_session(&SessionMeta {
                id: "sess-skill-lifecycle".into(),
                agent: Some("act".into()),
                model: Some("m".into()),
                created_at: 0,
                updated_at: 0,
                ..SessionMeta::default()
            })
            .await
            .unwrap();
        let probe = Arc::new(RetryProbeStore {
            attempts: AtomicUsize::new(0),
            fail_first,
            inner,
        });
        let arc: Arc<dyn Store> = probe.clone();
        (probe, arc)
    }

    /// Transient store failure: the durable clear retries (100/200ms) and
    /// lands on the third attempt; a recovered clear stays quiet (no Status
    /// noise) and the row is really cleared.
    #[tokio::test]
    async fn clear_retries_through_transient_store_failure() {
        let (probe, store) = probe_store(2).await;
        let s = make_session(Some(store));
        s.set_skill(Some("BODY".into()));

        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;

        assert_eq!(
            probe.attempts.load(Ordering::SeqCst),
            3,
            "2 failures + 1 successful retry"
        );
        assert!(s.skill_prompt_cloned().is_none(), "memory cleared");
        assert!(
            probe.inner
                .get_session("sess-skill-lifecycle")
                .await
                .unwrap()
                .and_then(|m| m.skill)
                .is_none(),
            "clear landed after retry"
        );
        assert!(
            evs.iter().all(|e| !matches!(e, SessionEvent::Status(_))),
            "recovered clear must not emit Status: {evs:?}"
        );
    }

    /// Persistent store failure: after the retries give up, the failure is
    /// surfaced as a visible Status event — never silently swallowed, which
    /// is what let the stale row re-arm every later run in the first place.
    #[tokio::test]
    async fn persistent_store_failure_surfaces_status() {
        let (probe, store) = probe_store(usize::MAX).await;
        let s = make_session(Some(store));
        s.set_skill(Some("BODY".into()));

        let mut evs: Vec<SessionEvent> = Vec::new();
        clear_on_run_end(&s, &mut |e| evs.push(e)).await;

        assert_eq!(
            probe.attempts.load(Ordering::SeqCst),
            3,
            "bounded retries, no infinite loop"
        );
        assert!(s.skill_prompt_cloned().is_none(), "memory still cleared");
        assert!(
            evs.iter().any(|e| matches!(e, SessionEvent::Status(t)
                if t.contains("persisted skill clear failed"))),
            "failure must be visible: {evs:?}"
        );
    }
}
