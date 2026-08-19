//! Plan-phase state: lifecycle helpers + durable mirror.
//!
//! A plan "phase" spans entry into plan mode until the plan is handed off to
//! act (`handoff` / `/act_clear_context`). Two pieces of state ride on
//! [`SessionState`] and must stay in lockstep between memory and the store:
//!
//! - [`SessionState::plan_snapshot`](crate::SessionState::plan_snapshot):
//!   the final plan text captured by compaction while it is still an
//!   assistant message. Once the plan is folded into the user-role summary
//!   head, `final_plan_text` can no longer find it — the snapshot is what
//!   `handoff` falls back to so a compacted plan session still hands the
//!   plan forward instead of silently degrading to a plain mode swap.
//! - [`SessionState::plan_input_count`](crate::SessionState::plan_input_count):
//!   requirements submitted in the current phase. Beyond the read-only
//!   reminder tag it is the durable arming signal: persisted to the store so
//!   a restarted / re-opened TUI re-arms the Shift+Tab plan→act handoff.
//!
//! Lifecycle (split): switching *to* plan mode resets ONLY the input counter
//! ([`SessionState::reset_plan_phase`]) — the snapshot survives the plain
//! re-entry, so a plan→act→plan toggle with no new requirement still owns the
//! previous phase's plan. The snapshot retires when a NEW requirement is
//! recorded (`maybe_tag_plan_prompt`, alongside the counter increment): a
//! failed or cancelled new-requirement turn must not leave the stale plan
//! armed (the `ecce7b0` anti-fabrication guard). The snapshot is captured per
//! plan turn by `SessionState::record`, rescued by compaction and consumed by
//! `after_handoff`.

use crate::SessionState;
use opencoder_core::message::now_ms;
use opencoder_core::{AgentKind, Message, Role};

impl SessionState {
    /// Reset the plan-phase input counter, re-arming the read-only reminder
    /// tag. Called when switching *to* plan mode. The `plan_snapshot`
    /// deliberately SURVIVES this reset: a plan→act→plan toggle with no new
    /// requirement submitted still owns the previous phase's plan, so
    /// `/act_clear_context` and the Shift+Tab handoff can carry it forward.
    /// The snapshot retires only when a new requirement is recorded
    /// (`maybe_tag_plan_prompt`) or the plan is consumed (`after_handoff`).
    pub fn reset_plan_phase(&mut self) {
        self.plan_input_count = 0;
    }

    /// Best-effort persist of the plan-phase state (`plan_input_count` +
    /// `plan_snapshot` mirror) so a resumed session re-arms plan-phase
    /// affordances (TUI Shift+Tab handoff, /act_clear_context provenance
    /// gate) and still finds a compaction-folded plan. Errors are logged,
    /// never fatal — this is bookkeeping, not transcript data.
    pub async fn persist_plan_phase(&self) {
        let store = match self.store.clone() {
            Some(s) => s,
            None => return,
        };
        let mut patch = opencoder_store::SessionPatch {
            plan_input_count: Some(self.plan_input_count as i64),
            updated_at: Some(now_ms()),
            ..Default::default()
        };
        match &self.plan_snapshot {
            Some(snap) => patch.plan_snapshot = Some(snap.clone()),
            None => patch.clear_plan_snapshot = true,
        }
        if let Err(e) = store.update_session(&self.id, &patch).await {
            tracing::warn!(session_id = %self.id, error = %e, "persist plan phase failed");
        }
    }
}

/// Candidate plan snapshot for a recorded message: `Some(text)` when the
/// session is in plan mode and the message is a real (non-synthetic)
/// assistant turn carrying non-empty text — i.e. the current plan phase just
/// produced plan content. Pure by design: `SessionState::record` applies it
/// (and persists the mirror) so the snapshot is phase-bounded — it can only
/// ever hold text produced while the plan agent was actually answering in
/// this phase, never an earlier act-mode reply scraped from the transcript.
pub fn plan_snapshot_update(kind: &AgentKind, msg: &Message) -> Option<String> {
    if *kind != AgentKind::Plan {
        return None;
    }
    if msg.role != Role::Assistant || msg.synthetic {
        return None;
    }
    let text = msg.text();
    (!text.trim().is_empty()).then_some(text)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_plan_session() -> SessionState {
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "plan-phase-test",
            resolve_agent("plan").unwrap(),
            Config::default(),
            client,
            PathBuf::from("."),
        )
    }

    fn assistant_text(id: &str, text: &str) -> Message {
        let mut m = Message::assistant(id);
        m.blocks.push(opencoder_core::ContentBlock::text(text));
        m
    }

    #[test]
    fn snapshot_update_captures_plan_assistant_text() {
        let kind = resolve_agent("plan").unwrap().kind;
        assert_eq!(
            plan_snapshot_update(&kind, &assistant_text("a1", "## Plan")),
            Some("## Plan".to_string())
        );
    }

    #[test]
    fn snapshot_update_rejects_act_kind_user_and_synthetic() {
        let plan = resolve_agent("plan").unwrap().kind;
        let act = resolve_agent("act").unwrap().kind;
        let user = Message::user("u1", "text");
        let mut synthetic = assistant_text("a2", "summary head");
        synthetic.synthetic = true;
        let empty = Message::assistant("a3");
        assert_eq!(plan_snapshot_update(&act, &assistant_text("a1", "x")), None);
        assert_eq!(plan_snapshot_update(&plan, &user), None);
        assert_eq!(plan_snapshot_update(&plan, &synthetic), None);
        assert_eq!(plan_snapshot_update(&plan, &empty), None);
    }

    #[test]
    fn reset_plan_phase_resets_counter_but_keeps_snapshot() {
        let mut s = make_plan_session();
        s.plan_input_count = 3;
        s.plan_snapshot = Some("## Plan".into());
        s.reset_plan_phase();
        assert_eq!(s.plan_input_count, 0, "counter must reset");
        assert_eq!(
            s.plan_snapshot.as_deref(),
            Some("## Plan"),
            "snapshot survives the plain switch to plan; it retires only on a \
             new requirement (maybe_tag_plan_prompt) or handoff"
        );
    }

    #[test]
    fn after_handoff_consumes_snapshot() {
        let mut s = make_plan_session();
        s.plan_snapshot = Some("## Plan".into());
        s.plan_input_count = 2;
        s.after_handoff(7, "## Plan".into());
        assert_eq!(s.handoff_plan, Some("## Plan".into()));
        assert_eq!(s.plan_snapshot, None, "snapshot consumed by handoff");
        assert_eq!(s.plan_input_count, 0, "counter consumed by handoff");
    }

    #[tokio::test]
    async fn persist_plan_phase_round_trip() {
        let store: Arc<dyn opencoder_store::Store> =
            Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
        let mut s = make_plan_session();
        s.store = Some(store.clone());
        let meta = opencoder_store::SessionMeta {
            id: s.id.clone(),
            ..Default::default()
        };
        store.create_session(&meta).await.unwrap();
        s.plan_input_count = 2;
        s.plan_snapshot = Some("## Plan".into());
        s.persist_plan_phase().await;

        let meta = store.get_session(&s.id).await.unwrap().unwrap();
        assert_eq!(meta.plan_snapshot.as_deref(), Some("## Plan"));
        assert_eq!(meta.plan_input_count, 2);

        // Consumed state persists as cleared/zero.
        s.after_handoff(3, "## Plan".into());
        s.persist_plan_phase().await;
        let meta = store.get_session(&s.id).await.unwrap().unwrap();
        assert_eq!(meta.plan_snapshot, None);
        assert_eq!(meta.plan_input_count, 0);
    }
}
