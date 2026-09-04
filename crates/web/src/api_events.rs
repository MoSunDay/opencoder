//! SSE events endpoints, split out of `api.rs` to respect the file-size
//! budget: `GET /events` (persisted replay + live broadcast with two-tier
//! dedup, subscriber-slot drop guard) and `GET /seq` (reconnect cursor).
//! Re-exported through `crate::api` so handler paths stay stable.
//!
//! Pre-subscribe gap 桥接：客户端在 POST /prompt 之后才建立 SSE 连接，事件
//! 若在 subscribe 之前广播、且回放查询 `events_after` 执行时仍未落库
//! （event flusher 对 delta 攒批滞后），既不在直播流也不在回放里，对该连接
//! 永久丢失。`SessionHandle::subscribe_recent` 在 subscribe 原子地附赠一份
//! 近期广播 ring 快照，本 handler 把其中未被回放覆盖（指纹/seq 去重后仍
//! 存活）的条目补发在回放之后，弥合该窗口。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::handle::{SessionHandle, SseEvt};
use crate::AppState;

#[derive(Deserialize, Default)]
pub struct EventsQuery {
    pub after: Option<i64>,
}

/// SSE stream: replay persisted events `after` the cursor, then forward the
/// live broadcast. Slow clients skip lagged events (backpressure never blocks
/// the runner); a missing live handle still yields the replay window.
///
/// The cursor comes from `?after=` or, when absent, the SSE-standard
/// `Last-Event-ID` header (unparseable values ignored → fall back to 0).
pub async fn get_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    // `Last-Event-ID` has no constant in the `http` crate; parse by name.
    let header_after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let after = q.after.or(header_after).unwrap_or(0);

    // Reject non-existent sessions: otherwise a get-or-created handle would
    // subscribe to a broadcast that never fires, hanging the SSE stream forever.
    match state.store.get_session(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("session not found"),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    }

    // Subscribe FIRST, then query persisted events. This closes the race where
    // an event broadcast between query and subscribe is lost (not yet
    // persisted at query time, not received via broadcast). With subscribe-first
    // every post-subscribe broadcast is captured by the live stream; any overlap
    // with the replay window is deduplicated below.
    //
    // 只在 map 锁内拿 handle Arc 并占订阅位；ring 快照（`subscribe_recent`，
    // 持 handle 自己的 recent 锁）放到 map 锁释放之后做，避免嵌套持锁的
    // 锁序问题。
    let (handle, created) = {
        let mut map = state.handles.lock().await;
        let created = !map.contains_key(&id);
        let handle = map.entry(id.clone()).or_insert_with(SessionHandle::new);
        // Track this subscriber so the handle this request may have created is
        // evicted (see `release_events_subscriber`) once everyone disconnects.
        handle.subscribers.fetch_add(1, Ordering::SeqCst);
        (Arc::clone(handle), created)
    };
    let (rx, ring) = handle.subscribe_recent();

    // P0-1: Capture the persisted-seq baseline BEFORE querying `events_after`.
    // `last_event_seq` returns the current max persisted seq; reading it AFTER
    // `events_after` (as the original code did) guarantees `baseline >=
    // max(seq)`, so the `seq > baseline` filter below is ALWAYS false and `seen`
    // (the tier-(2) content-dedup set) stays permanently empty. Snapshotting it
    // here — immediately after subscribing — means any event persisted in the
    // window between this snapshot and the `events_after` query (seq > baseline)
    // is a genuine subscribe/query overlap-window event that must seed `seen`.
    let baseline = state.store.last_event_seq(&id).await.unwrap_or(-1);

    let persisted: Vec<SseEvt> = state
        .store
        .events_after(&id, after)
        .await
        .map(|records| {
            records
                .into_iter()
                .map(|r| SseEvt {
                    kind: r
                        .sse_kind
                        .clone()
                        .unwrap_or_else(|| crate::api::event_kind_str(r.kind).to_string()),
                    data: r.payload,
                    ts: r.ts,
                    seq: r.seq,
                })
                .collect()
        })
        .unwrap_or_default();

    // Dedup live broadcast events against the replayed (persisted) window:
    // two-tier decision (exact seq, then content fingerprint) + overlap-window
    // seeding and the first-forwarded-`done` TTL live in `sse_dedup`.
    let max_replay_seq: i64 = persisted.iter().filter_map(|e| e.seq).max().unwrap_or(-1);
    let seen = crate::sse_dedup::seed_seen(&persisted, baseline);

    // Pre-subscribe gap 桥接：ring 里「已落库」的条目上面 replay 已覆盖
    // （指纹/seq 去重消费掉），「未落库」的条目（flusher 攒批滞后）在这里
    // 补发。顺序安全：flusher 单通道 FIFO 且结构性事件会先冲刷挂起 delta，
    // 因此任何未落库 ring 条目的发射序必然晚于全部已落库条目，直接接在
    // replay 之后即保持全局时序。
    //
    // 桥接用独立的多重集（`seed_bridge_seen`，覆盖整个回放窗口而非仅重叠
    // 窗口）：ring 里 subscribe 前就已落库的条目 seq <= baseline，若沿用
    // `seen` 的 baseline 过滤将无法命中指纹而被补发，与 replay 双发；该
    // 集合只在此处同步消费、不接触直播流，无 P0-1 历史指纹误吞直播的风险。
    let bridge_seen = crate::sse_dedup::seed_bridge_seen(&persisted);
    let bridged: Vec<SseEvt> = ring
        .into_iter()
        .filter(|e| crate::sse_dedup::forward_live(e, &bridge_seen, max_replay_seq))
        .collect();

    let replay = futures::stream::iter(persisted).chain(futures::stream::iter(bridged));
    let live =
        tokio_stream::wrappers::BroadcastStream::new(rx)
            .filter_map(|r| async move { crate::api::map_broadcast_result(r) })
            .filter_map({
                let seen = Arc::clone(&seen);
                move |evt| {
                    let seen = Arc::clone(&seen);
                    async move {
                        crate::sse_dedup::forward_live(&evt, &seen, max_replay_seq).then_some(evt)
                    }
                }
            });
    let merged = replay.chain(live).map(|evt| {
        let data = serde_json::to_string(&evt.data).unwrap_or_else(|_| "{}".into());
        // The persisted seq becomes the SSE `id:` field — the reconnect cursor
        // `Last-Event-ID` replays from. The builder consumes self, so the
        // optional id is applied with a reassignment (live frames without a
        // seq yet simply carry no id).
        let mut ev = Event::default().event(evt.kind).data(data);
        if let Some(seq) = evt.seq {
            ev = ev.id(seq.to_string());
        }
        Ok::<_, std::convert::Infallible>(ev)
    });

    // Wrap in a drop guard so that when the client disconnects (or the stream
    // ends) this request's subscriber slot is released and, if it created the
    // handle and nothing remains, the handle is evicted — preventing unbounded
    // handle-map growth on a long-running server.
    let guarded = crate::handle::DropGuardStream::new(merged, move || {
        crate::handle::release_events_subscriber(state.handles.clone(), id, created)
    });

    Sse::new(guarded)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Highest persisted event seq for a session (0 if none). A remote client uses
/// this to snapshot before posting a prompt so it only streams the events
/// produced by its own turn.
pub async fn get_event_seq(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    // Aligned with get_events: an unknown session is a 404, not a synthetic
    // `{seq: 0}` — a truncated id used to look like "no events yet" and made
    // the client replay from 0 (a full false transcript) instead of failing.
    match state.store.get_session(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_404("session not found"),
        Err(e) => return error_500(format!("get_session: {e:#}")),
    }
    let seq = state.store.last_event_seq(&id).await.unwrap_or(0);
    Json(json!({ "id": id, "seq": seq })).into_response()
}

fn error_404(msg: &str) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn error_500(msg: String) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
