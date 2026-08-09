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
