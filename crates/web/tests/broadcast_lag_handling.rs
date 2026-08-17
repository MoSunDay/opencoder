//! P0-2: a `BroadcastStream` lag must NOT be silently dropped. Previously
//! `.filter_map(|r| r.ok())` turned `Err(Lagged(n))` into `None`, which could
//! swallow a terminal `done`/`error` event and freeze the UI forever. Now the
//! lag is surfaced as a synthetic `error` event so the client knows to re-sync.

use opencoder_core::SseEvt;
use opencoder_web::api::map_broadcast_result;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

#[test]
fn ok_event_passes_through_unchanged() {
    let evt = SseEvt {
        kind: "done".into(),
        data: serde_json::json!({}),
        ts: 1,
        seq: None,
    };
    let out = map_broadcast_result(Ok(evt.clone())).unwrap();
    assert_eq!(out.kind, "done");
}

#[test]
fn lagged_is_surfaced_as_error_not_dropped() {
    // 500 dropped events.
    let out = map_broadcast_result(Err(BroadcastStreamRecvError::Lagged(500))).unwrap();
    assert_eq!(out.kind, "error");
    let msg = out
        .data
        .get("error")
        .and_then(|v| v.as_str())
        .expect("error event must carry an `error` string");
    assert!(msg.contains("lag"), "msg should mention lag, got: {msg}");
    assert!(
        msg.contains("500"),
        "msg should mention the count, got: {msg}"
    );
}
