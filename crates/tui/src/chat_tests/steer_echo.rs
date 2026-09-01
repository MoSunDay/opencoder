use super::super::*;

/// SteerConsumed echoes a `ChatBlock::User` block into the transcript at
/// consume time (turn boundary) and drops the consumed row from the pending
/// mirror. The block is NOT pushed at admit time — it only appears when the
/// steer actually starts executing.
#[test]
fn steer_consumed_echoes_marker_and_drops_row() {
    let mut v = ChatView::default();
    v.steer_items.push((7, "redirect here".into()));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 7,
        text: "redirect here".into(),
    });

    // The consumed row is removed from the mirror.
    assert!(
        v.steer_items.is_empty(),
        "SteerConsumed must drop the consumed row"
    );
    // A ChatBlock::User with the consumed prompt is pushed at consume time.
    assert!(
        block_text(&v).contains("User:"),
        "SteerConsumed must echo the User tag at consume time"
    );
    assert!(
        block_text(&v).contains("redirect here"),
        "SteerConsumed must echo the consumed prompt body"
    );
}

/// SteerConsumed for an unknown seq is a clean no-op (mirror untouched,
/// no spurious marker pushed).
#[test]
fn steer_consumed_unknown_seq_is_noop() {
    let mut v = ChatView::default();
    v.steer_items.push((9, "keep me".into()));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 999,
        text: String::new(),
    });

    assert_eq!(v.steer_items.len(), 1, "unknown seq must retain the row");
    assert!(!block_text(&v).contains("User:"));
}

/// A bare control command consumed from the steer boundary echoes NOTHING:
/// the event carries empty text (applied inline, nothing recorded) and the
/// mirror fallback must not resurrect the raw command either — the mirror row
/// is still dropped by seq.
#[test]
fn steer_consumed_bare_control_command_echoes_nothing() {
    let mut v = ChatView::default();
    v.steer_items.push((7, "/plan".into()));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 7,
        text: String::new(),
    });
    assert!(
        v.steer_items.is_empty(),
        "the mirror row is still dropped by seq"
    );
    assert!(
        !block_text(&v).contains("User:"),
        "a bare control command must not echo a user block"
    );
    // Legacy persisted events carry the raw prefix — the display layer
    // normalizes them to the model-facing tail.
    let mut v = ChatView::default();
    v.steer_items.push((8, "/plan review".into()));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 8,
        text: "/plan review".into(),
    });
    let text = block_text(&v);
    assert!(text.contains("review"), "compound tail is echoed: {text}");
    assert!(
        !text.contains("/plan"),
        "the command token itself must never be echoed: {text}"
    );
    // Mirror fallback (empty event text) normalizes the same way.
    let mut v = ChatView::default();
    v.steer_items.push((9, "/plan finish the summary".into()));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 9,
        text: String::new(),
    });
    let text = block_text(&v);
    assert!(text.contains("finish the summary"), "tail echoed: {text}");
    assert!(!text.contains("/plan"), "token suppressed: {text}");
}
