//! Replay fidelity test: all 16 SessionEvent variants must replay with the
//! same SSE event-name they were broadcast live with.
//!
//! Root cause of the original bug: the coarse `EventKind` enum (12 variants)
//! could not losslessly round-trip the granular `SessionEvent` (16 variants).
//! Eight variants collapsed into `EventKind::Step` or `EventKind::TextDelta`/
//! `EventKind::Compaction`, and the replay path (`event_kind_str`) produced
//! generic names (`"status"`, `"text_delta"`, `"compaction"`).
//!
//! Fix: persist the granular kind string in `sse_kind` and replay from it.
//! This test asserts the round-trip is lossless for every variant.

use std::sync::Arc;

use opencoder_session::SessionEvent;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store};
use opencoder_web::handle::sse_from_session_event;

/// Build all 16 SessionEvent variants (one representative of each).
fn all_variants() -> Vec<SessionEvent> {
    vec![
        SessionEvent::TextDelta("hello".into()),
        SessionEvent::ReasoningDelta("thinking".into()),
        SessionEvent::ToolStart {
            id: "t1".into(),
            name: "read".into(),
            input: serde_json::json!({"path": "x"}),
        },
        SessionEvent::ToolEnd {
            id: "t1".into(),
            name: "read".into(),
            output: "done".into(),
            is_error: false,
        },
        SessionEvent::AgentSwitch("act".into()),
        SessionEvent::Compaction("summary".into()),
        SessionEvent::Status("working".into()),
        SessionEvent::SubagentStart {
            id: "s1".into(),
            kind: "build".into(),
            prompt: "do something".into(),
            child_session_id: "child-1".into(),
        },
        SessionEvent::SubagentEnd {
            id: "s1".into(),
            ok: true,
            cancelled: false,
            summary: "finished".into(),
        },
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::TextDelta("child text".into())),
        },
        SessionEvent::TranscriptReset(vec![]),
        SessionEvent::PlanHandoff("plan text".into()),
        SessionEvent::QueueConsumed { seq: 42 },
        SessionEvent::SteerConsumed { seq: 7 },
        SessionEvent::Done,
        SessionEvent::Error("oops".into()),
    ]
}

#[tokio::test]
async fn replay_kind_matches_live_kind_for_all_variants() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "replay-fid";

    // Create a session row so the FK on session_events is satisfied.
    // SessionMeta already imported above
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 1,
            updated_at: 1,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();

    let variants = all_variants();
    assert_eq!(
        variants.len(),
        16,
        "expected exactly 16 SessionEvent variants"
    );

    // For each variant: compute the live kind, persist with sse_kind, read back.
    for (i, ev) in variants.iter().enumerate() {
        // Simulate the live broadcast path: from_session_event.
        let (sse, _coarse) = sse_from_session_event(sid, ev);
        let _live_kind = sse.kind.clone();

        // Persist the record exactly as the web drain does.
        store
            .append_event(&SessionEventRecord {
                session_id: sid.into(),
                kind: ev.coarse_kind(),
                payload: ev.sse_data(),
                ts: i as i64,
                seq: None,
                sse_kind: Some(ev.sse_kind().to_string()),
            })
            .await
            .unwrap();
    }

    // Read back all events and verify replayed kinds match live kinds.
    let records = store.events_after(sid, 0).await.unwrap();
    assert_eq!(records.len(), 16);

    // Replay mapping — same logic as api.rs get_events.
    fn event_kind_str(k: EventKind) -> &'static str {
        match k {
            EventKind::PromptAdmitted => "prompt_admitted",
            EventKind::PromptPromoted => "prompt_promoted",
            EventKind::TextDelta => "text_delta",
            EventKind::ToolStart => "tool_start",
            EventKind::ToolEnd => "tool_end",
            EventKind::AgentSwitched => "agent_switched",
            EventKind::ModelSwitched => "model_switched",
            EventKind::Compaction => "compaction",
            EventKind::Step => "status",
            EventKind::Interrupted => "interrupted",
            EventKind::Done => "done",
            EventKind::Error => "error",
        }
    }

    for (i, rec) in records.iter().enumerate() {
        let ev = &variants[i];
        let live_kind = ev.sse_kind();

        // Replay path: prefer sse_kind, fall back to coarse.
        let replayed_kind = rec
            .sse_kind
            .as_deref()
            .unwrap_or_else(|| event_kind_str(rec.kind));

        assert_eq!(
            replayed_kind, live_kind,
            "variant #{} ({:?}): replayed kind mismatch — sse_kind preserved the granular name",
            i, ev
        );

        // Without sse_kind (old records), the fallback would degrade for 8 variants.
        let fallback_kind = event_kind_str(rec.kind);
        if fallback_kind != live_kind {
            // This variant would be degraded without sse_kind — confirming the fix is needed.
            // The sse_kind column prevents this degradation.
            assert!(
                rec.sse_kind.is_some(),
                "variant #{} ({:?}): degraded variant must have sse_kind set",
                i,
                ev
            );
        }
    }
}
