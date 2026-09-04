//! Live-broadcast dedup for the SSE `/events` stream (`api::get_events`).
//!
//! Replay (persisted) events are always forwarded; a live broadcast may
//! duplicate an event that was persisted between the handler's subscribe and
//! its replay query (the overlap window). This module owns the two-tier
//! decision of whether a live event is such a duplicate, plus the seeding of
//! the content-fingerprint multiset from the overlap window. Pure functions
//! over `(SseEvt, seen, max_replay_seq)` — no state beyond the shared `seen`
//! set.
//!
//! 指纹集合是「计数多重集」而非 HashSet：同一 (kind, data) 内容的事件可能在
//! 种子窗口内出现多次（如若干条完全相同的 delta），HashSet 只保留一份指纹，
//! 只能吞掉一条重复、放行其余，造成双发；多重集按份数精确消耗——每一条重复
//! 广播消耗一份计数，计数归零才移除键。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::handle::SseEvt;

/// Content fingerprints of seeded events: `(sse kind, data JSON)` -> 份数.
pub(crate) type SeenFingerprints = Arc<Mutex<HashMap<(String, String), usize>>>;

/// Seed the fingerprint multiset from the overlap window only: replayed
/// events whose persisted `seq` exceeds the `baseline` snapshot taken BEFORE
/// the `events_after` query (P0-1). Pre-filling from the entire replay window
/// wrongly armed historical fingerprints: a live `done` (always `{}`, no seq)
/// colliding with ANY historical `done` was silently dropped, freezing the
/// UI (busy never resets, send disabled). Seeding from the true overlap
/// window keeps historical events from suppressing live ones.
///
/// 多重集语义：同内容条目按出现份数累加计数（见文件头注释）。
pub(crate) fn seed_seen(persisted: &[SseEvt], baseline: i64) -> SeenFingerprints {
    seed_filtered(persisted, |seq| seq > baseline)
}

/// 为 pre-subscribe gap 桥接（ring 补发）种子指纹多重集：不设 baseline
/// 过滤，回放窗口内每条持久化事件都计入一份。
///
/// 与 `seed_seen` 的区别是刻意的：桥接过滤器在 subscribe 之后同步地一次性
/// 消费 ring 快照，从不接触直播流，因此「历史指纹误吞直播事件」的 P0-1
/// 风险不存在；而 ring 里已落库条目（seq 可能 <= baseline，即 subscribe 前
/// 早已持久化的那部分）必须能被指纹命中才会被过滤，否则会与回放双发。
pub(crate) fn seed_bridge_seen(persisted: &[SseEvt]) -> SeenFingerprints {
    seed_filtered(persisted, |_| true)
}

/// 按 `keep` 过滤回放条目并按内容份数累加。
fn seed_filtered(persisted: &[SseEvt], keep: impl Fn(i64) -> bool) -> SeenFingerprints {
    let mut multiset: HashMap<(String, String), usize> = HashMap::new();
    for e in persisted.iter().filter(|e| e.seq.is_some_and(&keep)) {
        *multiset
            .entry((e.kind.clone(), e.data.to_string()))
            .or_insert(0) += 1;
    }
    Arc::new(Mutex::new(multiset))
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
        // 多重集消耗：命中（计数 > 0）即视为重复，计数减一；减到 0 移除键，
        // 保证同内容的 N 份重复各消耗一份、第 N+1 条同内容直播是真新事件。
        let hit = match guard.get_mut(&key) {
            Some(count) if *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        };
        if hit {
            guard.retain(|_, count| *count > 0);
            return false;
        }
        if evt.kind == "done" {
            guard.clear();
        }
    }
    true
}
