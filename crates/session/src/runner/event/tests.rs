use super::*;

/// `from_sse` is the exact inverse of `sse_kind()` + `sse_data()` for every
/// variant EXCEPT `TranscriptReset`, whose payload is `{}` on the wire
/// (the rebuilt message list cannot be carried over SSE and must be
/// re-fetched). Pin both the roundtrip and that documented lossiness.
#[test]
fn from_sse_roundtrips_all_variants() {
    let cases: Vec<SessionEvent> = vec![
        SessionEvent::LlmRoundStart {
            started_at_ms: 1234,
        },
        SessionEvent::LlmRoundEnd,
        SessionEvent::LlmAttemptReset,
        SessionEvent::LlmUsage {
            total_tokens: 123_456,
            input_tokens: 100_000,
            output_tokens: 23_456,
        },
        SessionEvent::TextDelta("hi".into()),
        SessionEvent::ReasoningDelta("think".into()),
        SessionEvent::ToolStart {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        },
        SessionEvent::ToolEnd {
            id: "t1".into(),
            name: "bash".into(),
            output: "done".into(),
            is_error: false,
            images: Vec::new(),
        },
        SessionEvent::ToolEnd {
            id: "t2".into(),
            name: "bash".into(),
            output: "boom".into(),
            is_error: true,
            images: Vec::new(),
        },
        SessionEvent::AgentSwitch("plan".into()),
        SessionEvent::ModelSwitch("openai/gpt-4o".into()),
        SessionEvent::Compaction("summary".into()),
        SessionEvent::CompactionDelta("cdelta".into()),
        SessionEvent::Status("running".into()),
        SessionEvent::SubagentStart {
            id: "s1".into(),
            kind: "explore".into(),
            prompt: "find x".into(),
            child_session_id: "child-1".into(),
        },
        SessionEvent::SubagentEnd {
            id: "s1".into(),
            ok: true,
            cancelled: false,
            summary: "found".into(),
        },
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::TextDelta("child text".into())),
        },
        SessionEvent::SidecarStart {
            id: "sidecar-1".into(),
            question: "progress?".into(),
        },
        SessionEvent::SidecarChild {
            id: "sidecar-1".into(),
            ev: Box::new(SessionEvent::TextDelta("sidecar text".into())),
        },
        SessionEvent::SidecarTurn {
            id: "sidecar-1".into(),
            ok: true,
            answer: "half done".into(),
            elapsed_ms: 42,
            total_tokens: 1234,
            rounds: 1,
        },
        SessionEvent::TranscriptReset(vec![Message::assistant("m1")]),
        SessionEvent::QueueConsumed {
            seq: 7,
            text: "q".into(),
        },
        SessionEvent::SteerConsumed {
            seq: 9,
            text: "s".into(),
        },
        SessionEvent::AutoPilot {
            phase: ApPhase::Plan,
            iteration: 0,
        },
        SessionEvent::Done,
        SessionEvent::Error("kaboom".into()),
    ];
    let mut kinds: Vec<&str> = cases.iter().map(|e| e.sse_kind()).collect();
    kinds.sort();
    kinds.dedup();
    assert_eq!(
        kinds.len(),
        25,
        "expected all 25 unique kinds, got {kinds:?}"
    );

    for ev in &cases {
        let kind = ev.sse_kind();
        let data = ev.sse_data();
        let back = SessionEvent::from_sse(kind, data.clone())
            .unwrap_or_else(|| panic!("from_sse returned None for kind={kind} data={data}"));
        if matches!(ev, SessionEvent::TranscriptReset(_)) {
            // documented lossiness: no messages on the wire
            assert!(matches!(back, SessionEvent::TranscriptReset(ref v) if v.is_empty()));
        } else {
            assert_eq!(
                serde_json::to_string(&back).unwrap(),
                serde_json::to_string(ev).unwrap(),
                "roundtrip mismatch for kind={kind}"
            );
        }
    }
}

#[test]
fn from_sse_unknown_kind_is_none() {
    assert!(SessionEvent::from_sse("no_such_kind", serde_json::json!({})).is_none());
}

/// Backward compatibility: llm_usage payloads persisted before the
/// input/output split must still deserialize (split fields default to 0)
/// — both on the SSE wire form and the direct enum form used by the store.
#[test]
fn llm_usage_old_payload_defaults_split_fields_to_zero() {
    let ev = SessionEvent::from_sse("llm_usage", serde_json::json!({ "total_tokens": 42 }))
        .expect("old llm_usage payload must parse");
    match ev {
        SessionEvent::LlmUsage {
            total_tokens,
            input_tokens,
            output_tokens,
        } => {
            assert_eq!(total_tokens, 42);
            assert_eq!(input_tokens, 0);
            assert_eq!(output_tokens, 0);
        }
        other => panic!("expected LlmUsage, got {other:?}"),
    }
    let stored: SessionEvent = serde_json::from_str(r#"{"LlmUsage":{"total_tokens":42}}"#).unwrap();
    assert!(matches!(
        stored,
        SessionEvent::LlmUsage {
            total_tokens: 42,
            input_tokens: 0,
            output_tokens: 0,
        }
    ));
}

#[test]
fn from_sse_missing_field_is_none() {
    // tool_start without the required `name` field
    assert!(SessionEvent::from_sse("tool_start", serde_json::json!({"id":"x"})).is_none());
}

#[test]
fn queue_consumed_carries_text_through_sse() {
    let ev = SessionEvent::QueueConsumed {
        seq: 5,
        text: "hello queued".into(),
    };
    let kind = ev.sse_kind();
    assert_eq!(kind, "queue_consumed");
    let data = ev.sse_data();
    assert_eq!(data["text"], "hello queued");
    assert_eq!(data["seq"], 5);
    let back = SessionEvent::from_sse(kind, data).expect("roundtrip");
    match back {
        SessionEvent::QueueConsumed { seq, text } => {
            assert_eq!(seq, 5);
            assert_eq!(text, "hello queued");
        }
        other => panic!("expected QueueConsumed, got {other:?}"),
    }
}

#[test]
fn steer_consumed_carries_text_through_sse() {
    let ev = SessionEvent::SteerConsumed {
        seq: 9,
        text: "steered away".into(),
    };
    let kind = ev.sse_kind();
    assert_eq!(kind, "steer_consumed");
    let data = ev.sse_data();
    assert_eq!(data["text"], "steered away");
    assert_eq!(data["seq"], 9);
    let back = SessionEvent::from_sse(kind, data).expect("roundtrip");
    match back {
        SessionEvent::SteerConsumed { seq, text } => {
            assert_eq!(seq, 9);
            assert_eq!(text, "steered away");
        }
        other => panic!("expected SteerConsumed, got {other:?}"),
    }
}

#[test]
fn queue_consumed_without_text_field_is_backward_compatible() {
    // Old persisted events predate the `text` field. A queue_consumed SSE
    // payload without the key must still deserialize (defaults to empty).
    let data = serde_json::json!({ "seq": 11 });
    let ev = SessionEvent::from_sse("queue_consumed", data).expect("old event");
    match ev {
        SessionEvent::QueueConsumed { seq, text } => {
            assert_eq!(seq, 11);
            assert!(text.is_empty(), "missing text must default to empty");
        }
        other => panic!("expected QueueConsumed, got {other:?}"),
    }
}

#[test]
fn steer_consumed_without_text_field_is_backward_compatible() {
    let data = serde_json::json!({ "seq": 13 });
    let ev = SessionEvent::from_sse("steer_consumed", data).expect("old event");
    match ev {
        SessionEvent::SteerConsumed { seq, text } => {
            assert_eq!(seq, 13);
            assert!(text.is_empty());
        }
        other => panic!("expected SteerConsumed, got {other:?}"),
    }
}

#[test]
fn tool_end_images_roundtrip_through_sse() {
    let ev = SessionEvent::ToolEnd {
        id: "img-1".into(),
        name: "view_image".into(),
        output: "Loaded image: cat.png".into(),
        is_error: false,
        images: vec![
            "data:image/png;base64,iVBORw0KGgo=".into(),
            "https://example.com/photo.jpg".into(),
        ],
    };
    let kind = ev.sse_kind();
    let data = ev.sse_data();
    let back = SessionEvent::from_sse(kind, data).expect("roundtrip");
    match back {
        SessionEvent::ToolEnd { images, .. } => {
            assert_eq!(images.len(), 2, "images must survive roundtrip");
            assert_eq!(images[0], "data:image/png;base64,iVBORw0KGgo=");
            assert_eq!(images[1], "https://example.com/photo.jpg");
        }
        other => panic!("expected ToolEnd, got {other:?}"),
    }
}

#[test]
fn tool_end_without_images_field_is_backward_compatible() {
    // Old persisted events predate the `images` field. A tool_end SSE
    // payload without the key must still deserialize (defaults to empty).
    let data = serde_json::json!({
        "id": "old",
        "name": "bash",
        "output": "done",
        "is_error": false,
    });
    let ev = SessionEvent::from_sse("tool_end", data).expect("old event");
    match ev {
        SessionEvent::ToolEnd { images, .. } => {
            assert!(
                images.is_empty(),
                "missing images field must default to empty"
            );
        }
        other => panic!("expected ToolEnd, got {other:?}"),
    }
}

/// P2-6: `from_sse` must saturate `iteration` to u32::MAX when the JSON
/// value exceeds u32's range (e.g. u64::MAX). The old `as u32` cast
/// silently wrapped to a small number, producing a wrong iteration index.
#[test]
fn from_sse_autopilot_large_iteration_saturates() {
    let data = serde_json::json!({
        "phase": "act",
        "iteration": u64::MAX,
    });
    let ev = SessionEvent::from_sse("autopilot", data).expect("must parse");
    match ev {
        SessionEvent::AutoPilot { iteration, .. } => {
            assert_eq!(
                iteration,
                u32::MAX,
                "iteration must saturate to u32::MAX, not wrap"
            );
        }
        other => panic!("expected AutoPilot, got {other:?}"),
    }
}

/// Exactly the three Sidecar* frames are persistence-gated; the bare
/// `LlmUsage` (the sidecar's cost-accounting channel) and every
/// Subagent* frame must stay persistable.
#[test]
fn is_sidecar_frame_marks_exactly_the_sidecar_variants() {
    let sidecar: Vec<SessionEvent> = vec![
        SessionEvent::SidecarStart {
            id: "sc".into(),
            question: "q".into(),
        },
        SessionEvent::SidecarChild {
            id: "sc".into(),
            ev: Box::new(SessionEvent::TextDelta("t".into())),
        },
        SessionEvent::SidecarTurn {
            id: "sc".into(),
            ok: true,
            answer: "a".into(),
            elapsed_ms: 1,
            total_tokens: 2,
            rounds: 1,
        },
    ];
    assert!(sidecar.iter().all(|e| e.is_sidecar_frame()));
    let keep: Vec<SessionEvent> = vec![
        SessionEvent::LlmUsage {
            total_tokens: 10,
            input_tokens: 7,
            output_tokens: 3,
        },
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::Done),
        },
        SessionEvent::Done,
    ];
    assert!(keep.iter().all(|e| !e.is_sidecar_frame()));
}
