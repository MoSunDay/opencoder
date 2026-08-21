//! `[tok cost]` state: LlmUsage accumulation (incl. subagent rounds),
//! provider-truth real-context tracking, and replay-time summation of
//! persisted assistant `usage`.

use super::super::*;

#[test]
fn llm_usage_events_accumulate() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 500_000,
        input_tokens: 400_000,
        output_tokens: 100_000,
    });
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 700_000,
        input_tokens: 650_000,
        output_tokens: 50_000,
    });
    assert_eq!(v.tokens_total, 1_200_000);
}

#[test]
fn llm_usage_accumulation_is_display_only_and_survives_reset() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 123,
        input_tokens: 100,
        output_tokens: 23,
    });
    v.apply(&SessionEvent::TranscriptReset(Vec::new()));
    // Lifetime accumulator is never cleared by a transcript reset.
    assert_eq!(v.tokens_total, 123);
    // Display-only: the value must not leak into message text/context.
    assert!(!block_text(&v).contains("tok cost"));
}

#[test]
fn subagent_child_usage_accumulates_into_parent_and_child() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::SubagentStart {
        id: "s1".into(),
        kind: "explore".into(),
        prompt: "find x".into(),
        child_session_id: "c1".into(),
    });
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 1_000,
        input_tokens: 900,
        output_tokens: 100,
    });
    v.apply(&SessionEvent::SubagentChild {
        id: "s1".into(),
        ev: Box::new(SessionEvent::LlmUsage {
            total_tokens: 300,
            input_tokens: 250,
            output_tokens: 50,
        }),
    });
    assert_eq!(
        v.tokens_total, 1_300,
        "parent lifetime cost includes subagent rounds"
    );
    // The child's context is NOT the parent's context: the forwarded round
    // must not touch the parent's real-context tracker.
    assert_eq!(v.real_context_tokens, Some(1_000));
    match v.blocks.last() {
        Some(ChatBlock::Subagent { view, .. }) => {
            assert_eq!(view.tokens_total, 300, "child keeps its own cost");
            assert_eq!(
                view.real_context_tokens,
                Some(300),
                "child tracks its own provider-truth context"
            );
        }
        other => panic!("expected subagent block, got {other:?}"),
    }
}

#[test]
fn real_context_tracks_latest_round_total_tokens() {
    let mut v = ChatView::default();
    assert_eq!(v.real_context_tokens, None, "starts without provider truth");
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 1_400,
        input_tokens: 1_200,
        output_tokens: 200,
    });
    assert_eq!(v.real_context_tokens, Some(1_400));
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 900,
        input_tokens: 700,
        output_tokens: 200,
    });
    assert_eq!(
        v.real_context_tokens,
        Some(900),
        "frozen at the latest completed round, not a sum"
    );
    // ctx uses the returned `total_tokens` verbatim, even when it differs
    // from input+output (e.g. cached/reasoning tokens counted in total).
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 2_000,
        input_tokens: 30,
        output_tokens: 20,
    });
    assert_eq!(v.real_context_tokens, Some(2_000));
    // Old payloads without the split fields deserialize input/output to 0 —
    // the returned total still carries the real context.
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 50,
        input_tokens: 0,
        output_tokens: 0,
    });
    assert_eq!(v.real_context_tokens, Some(50));
}

#[test]
fn real_context_survives_compaction_transcript_reset_and_model_switch() {
    let usage = SessionEvent::LlmUsage {
        total_tokens: 500,
        input_tokens: 400,
        output_tokens: 100,
    };
    let mut v = ChatView::default();
    v.apply(&usage);

    let mut c = v.clone();
    c.apply(&SessionEvent::Compaction("summary".into()));
    assert_eq!(
        c.real_context_tokens,
        Some(500),
        "compaction keeps the last real value until the next round reports"
    );

    let mut r = v.clone();
    r.apply(&SessionEvent::TranscriptReset(Vec::new()));
    assert_eq!(
        r.real_context_tokens,
        Some(500),
        "in-place reset keeps the stale value; rebuilds go through replay"
    );

    let mut m = v.clone();
    m.apply(&SessionEvent::ModelSwitch("openai/gpt-4o".into()));
    assert_eq!(
        m.real_context_tokens,
        Some(500),
        "model switch keeps the last real value until the new model reports"
    );
}

#[test]
fn model_switch_keeps_real_context_and_cost() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 7_000,
        input_tokens: 6_000,
        output_tokens: 1_000,
    });
    v.apply(&SessionEvent::ModelSwitch("openai/gpt-4o-mini".into()));
    assert_eq!(v.real_context_tokens, Some(7_000));
    assert_eq!(
        v.tokens_total, 7_000,
        "lifetime cost survives a model switch"
    );
}

#[test]
fn replay_sums_persisted_assistant_usage() {
    use opencoder_core::Message;
    let mut a1 = Message::assistant("a1");
    a1.usage.total_tokens = 1_000_000;
    let u1 = Message::user("u1", "hi");
    let mut a2 = Message::assistant("a2");
    a2.usage.total_tokens = 500_000;
    let mut no_usage = Message::assistant("a3");
    no_usage.usage.total_tokens = 0;
    let chat = crate::session_ui::replay_messages("act", &[a1, u1, a2, no_usage]);
    assert_eq!(
        chat.tokens_total, 1_500_000,
        "replay sums real usage across assistant messages (user/zero excluded)"
    );
    assert_eq!(
        chat.real_context_tokens,
        Some(500_000),
        "replay rebuilds provider truth from the latest non-zero usage"
    );
}
