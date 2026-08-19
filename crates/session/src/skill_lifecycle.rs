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
/// `active_skill_names`) and, best-effort, the store (`clear_skill: true`).
///
/// Idempotent guard: a skill-less session returns immediately WITHOUT a
/// store write, so plain (skill-less) runs never touch `sessions.updated_at`.
/// Store errors are swallowed — the in-memory clear already guarantees the
/// next run starts skill-less; the durable clear only matters for resume,
/// and a transient store hiccup must not mask the run's own result (same
/// best-effort contract as `autopilot::clear_injected_skill`).
pub(crate) async fn clear_on_run_end(session: &SessionState) {
    if session.skill_prompt_cloned().is_none() {
        return;
    }
    session.set_skill(None);
    session.set_active_skill_names(HashSet::new());
    if let Some(store) = &session.store {
        let _ = store
            .update_session(
                &session.id,
                &SessionPatch {
                    clear_skill: true,
                    updated_at: Some(now_ms()),
                    ..Default::default()
                },
            )
            .await;
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
    clear_on_run_end(session).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};
    use opencoder_store::{LibsqlStore, Store};

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

        clear_on_run_end(&s).await;

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

        clear_on_run_end(&s).await;

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
        clear_on_run_end(&s).await;
        clear_on_run_end(&s).await;
        assert!(s.skill_prompt_cloned().is_none());
    }

    /// Store-less sessions (TUI in-memory path) still clear in memory.
    #[tokio::test]
    async fn store_less_session_clears_memory_only() {
        let s = make_session(None);
        s.set_skill(Some("BODY".into()));
        clear_on_run_end(&s).await;
        assert!(s.skill_prompt_cloned().is_none());
    }
}
