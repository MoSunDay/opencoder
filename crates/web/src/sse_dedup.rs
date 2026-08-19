//! Live-broadcast dedup for the SSE `/events` stream (`api::get_events`).
//!
//! Replay (persisted) events are always forwarded; a live broadcast may
//! duplicate an event that was persisted between the handler's subscribe and
//! its replay query (the overlap window). This module owns the two-tier
//! decision of whether a live event is such a duplicate, plus the seeding of
//! the content-fingerprint set from the overlap window. Pure functions over
//! `(SseEvt, seen, max_replay_seq)` — no state beyond the shared `seen` set.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::handle::SseEvt;

/// Content fingerprints of overlap-window events: `(sse kind, data JSON)`.
pub(crate) type SeenFingerprints = Arc<Mutex<HashSet<(String, String)>>>;

/// Seed the fingerprint set from the overlap window only: replayed events
/// whose persisted `seq` exceeds the `baseline` snapshot taken BEFORE the
/// `events_after` query (P0-1). Pre-filling from the entire replay window
/// wrongly armed historical fingerprints: a live `done` (always `{}`, no seq)
/// colliding with ANY historical `done` was silently dropped, freezing the
/// UI (busy never resets, send disabled). Seeding from the true overlap
/// window keeps historical events from suppressing live ones.
pub(crate) fn seed_seen(persisted: &[SseEvt], baseline: i64) -> SeenFingerprints {
    Arc::new(Mutex::new(
        persisted
            .iter()
            .filter(|e| e.seq.is_some_and(|s| s > baseline))
            .map(|e| (e.kind.clone(), e.data.to_string()))
            .collect(),
    ))
}

/// Whether a live broadcast event must be forwarded (true) or is a duplicate
/// of the replayed window (false).
///
/// Every live event is broadcast with `seq: None` in this codebase
/// (persistence runs async via the event flusher; see
/// `sse_from_session_event`), while persisted/replayed events carry
/// `seq: Some(n)`. Dedup therefore runs in two tiers:
///  (1) If the live event DOES carry a persisted `seq`: drop it iff
///      `seq <= max_replay_seq`. This is exact and never collapses two
///      distinct events that merely share kind+payload (the H7 content-key
///      collision bug).
///  (2) The normal case — `seq: None`: an event broadcast after we subscribed
///      may also have been persisted before we queried (the overlap window).
///      Fall back to a content-fingerprint match; each fingerprint is consumed
///      on first match. Pure content dedup is unavoidable here because the
///      live copy has no seq to compare; tier (1) removes the collision risk
///      whenever a seq is available.
///
/// P2-4 fingerprint TTL: the first live `done` FORWARDED on the stream clears
/// the set — `done` is a run's last event, so once one is forwarded the
/// overlap window is deterministically over and any later content collision
/// is a genuine NEW event. Without the TTL, a fingerprint that never matched
/// a duplicate broadcast stays armed for the whole stream lifetime and
/// silently eats later live events with identical content.
pub(crate) fn forward_live(evt: &SseEvt, seen: &SeenFingerprints, max_replay_seq: i64) -> bool {
    // (1) Exact seq-based dedup when the live event carries a seq.
    if let Some(seq) = evt.seq {
        if seq <= max_replay_seq {
            return false;
        }
        if evt.kind == "done" {
            if let Ok(mut guard) = seen.lock() {
                guard.clear();
            }
        }
        return true;
    }
    // (2) Content overlap dedup for seq-less broadcasts.
    let key = (evt.kind.clone(), evt.data.to_string());
    if let Ok(mut guard) = seen.lock() {
        if guard.remove(&key) {
            return false;
        }
        if evt.kind == "done" {
            guard.clear();
        }
    }
    true
}
