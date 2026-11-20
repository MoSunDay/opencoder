//! Bounded persisted-replay window for the SSE `/events` endpoints.
//!
//! First-connect replay (`after=0`) used to load the ENTIRE persisted event
//! history per connection (`events_after` has no LIMIT): a long session ×
//! concurrent tabs / reconnect storms cloned the full history into memory per
//! connection, spiking server RSS. The store's `events_tail` reads the newest
//! window under a row + payload-byte budget, which matches the accepted client
//! semantic (SPA `sse.js`: "on reconnect, replay a bounded tail of the missed
//! window ... a capped tail still converges"); clients needing older history
//! page it explicitly via `/events-page`.

use opencoder_store::SessionEventRecord;

/// Row cap for one connection's replay window (newest events kept).
pub(crate) const REPLAY_MAX_EVENTS: u32 = 1000;
/// Payload byte cap (plus per-row overhead) for one connection's replay.
pub(crate) const REPLAY_BYTE_BUDGET: usize = 2 * 1024 * 1024;

/// Bounded newest-window replay (see client sse.js "capped tail converges").
///
/// Query errors map to an empty window, exactly like the previous
/// `events_after(...).unwrap_or_default()` call sites: replay is best-effort,
/// the live broadcast remains authoritative.
pub(crate) async fn bounded_replay(
    store: &dyn opencoder_store::Store,
    session: &str,
    after: i64,
) -> Vec<SessionEventRecord> {
    store
        .events_tail(session, after, REPLAY_MAX_EVENTS, REPLAY_BYTE_BUDGET)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(session, %error, "bounded replay query failed, sending live-only stream");
            opencoder_store::SessionEventPage {
                events: Vec::new(),
                more: false,
            }
        })
        .events
}
