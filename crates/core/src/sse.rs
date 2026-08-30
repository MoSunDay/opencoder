//! Shared SSE event envelope. The server (`opencoder-web`) emits these as the
//! wire format for its `/events` stream; the client (`opencoder-client`)
//! deserializes them back. Keeping the type in `opencoder-core` means both
//! sides share one definition and a payload shape change is a compile error
//! on both sides.

use serde::{Deserialize, Serialize};

/// A single SSE event. `kind` is the granular event-name string (mirrors
/// `SessionEvent::sse_kind`); `data` is the structured payload (mirrors
/// `SessionEvent::sse_data`). `ts` stays server-internal; a persisted `seq`
/// (replay frames, and hub broadcasts once the store write returned a seq) is
/// emitted as the SSE `id:` field — the `Last-Event-ID` reconnect cursor.
/// Event content itself is still reconstructed from `kind` + `data` alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseEvt {
    pub kind: String,
    pub data: serde_json::Value,
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}
