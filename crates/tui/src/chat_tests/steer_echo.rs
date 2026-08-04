use super::super::*;

/// SteerConsumed no longer echoes a `steer:` marker into the transcript.
/// The marker is now pushed at admit time (app.rs), so consuming a steer at
/// the turn boundary must only remove the row from the pending mirror and
/// leave the transcript untouched.
#[test]
fn steer_consumed_drops_row_without_marker() {
    let mut v = ChatView::default();
    v.steer_items.push((7, "redirect here".into()));
    v.apply(&SessionEvent::SteerConsumed { seq: 7 });

    // The consumed row is removed from the mirror.
    assert!(
        v.steer_items.is_empty(),
        "SteerConsumed must drop the consumed row"
    );
    // No `steer:` marker leaked into the transcript.
    assert!(
        !block_text(&v).contains("steer:"),
        "SteerConsumed must NOT push a marker — echoed at admit time"
    );
}

/// SteerConsumed for an unknown seq is a clean no-op (mirror untouched).
#[test]
fn steer_consumed_unknown_seq_is_noop() {
    let mut v = ChatView::default();
    v.steer_items.push((9, "keep me".into()));
    v.apply(&SessionEvent::SteerConsumed { seq: 999 });

    assert_eq!(v.steer_items.len(), 1, "unknown seq must retain the row");
    assert!(!block_text(&v).contains("steer:"));
}
