//! 回归测试：SSE「pre-subscribe gap 桥接」。
//!
//! 根因：daemon 的 drain 回调对每个事件同时 (a) 直播广播、(b) 走异步
//! event flusher 落库（delta 攒批 512 条 / 8KB 才 flush）。客户端在
//! POST /prompt 之后才建立 SSE 连接，事件若在「subscribe 之前广播」且
//! 「回放查询 events_after 执行时还未落库」，既不在直播流也不在回放里，
//! 对该连接永久丢失（实测 reasoning_delta 丢失导致直播态布局与 done 后
//! 快照重建不一致）。
//!
//! 修复：`SessionHandle` 增加近期广播环形缓冲（`recent`），`broadcast_evt`
//! 在同一把锁内「先入 ring 再发直播」；`get_events` 在 subscribe 原子地取
//! `(rx, ring 快照)`，把 ring 中未被回放覆盖（指纹/seq 去重后存活）的条目
//! 补发在回放之后。指纹集合为计数多重集：同内容事件出现 N 份时按份数精确
//! 消耗，HashSet 只留一份会把真实直播/补发事件误吞或放行错份数。
//!
//! 四个用例全部确定性构造（无真实竞态）：直接对预插入的 SessionHandle
//! 调 `broadcast_evt` 模拟「已广播未落库」，再调用
//! `opencoder_web::api::get_events` 消费 SSE 字节流计数。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use futures::StreamExt;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store};
use opencoder_web::handle::{SessionHandle, SseEvt};
use opencoder_web::AppState;
use serde_json::json;

/// 各用例唯一的 payload 标记，保证计数无歧义。
const EARLY_MARKER: &str = "__gap_early_persisted__";
const GAP_MARKER: &str = "__gap_unpersisted_delta__";
const DOUBLE_MARKER: &str = "__ring_and_persist__";
const PAIR_MARKER: &str = "__identical_pair__";
const LIVE_MARKER: &str = "__post_subscribe_live__";

async fn make_state() -> Arc<AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(AppState {
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

async fn seed(state: &AppState, sid: &str) {
    state
        .store
        .create_session(&SessionMeta {
            id: sid.into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
}

/// 预插入 SessionHandle 并返回其 Arc，模拟「会话已有广播通道」。get_events
/// 的 get-or-create 会命中同一个实例，因此 ring 快照可见。
async fn make_handle(state: &AppState, sid: &str) -> Arc<SessionHandle> {
    let handle = SessionHandle::new();
    state
        .handles
        .lock()
        .await
        .insert(sid.to_string(), handle.clone());
    handle
}

async fn append_event(
    state: &AppState,
    sid: &str,
    sse_kind: &str,
    payload: serde_json::Value,
    ts: i64,
) {
    state
        .store
        .append_event(&SessionEventRecord {
            session_id: sid.into(),
            kind: EventKind::Step,
            payload,
            ts,
            seq: None,
            sse_kind: Some(sse_kind.into()),
        })
        .await
        .unwrap();
}

/// 调用 get_events 并在有界窗口内消费 SSE 字节（直播流永不结束，靠
/// deadline 收口）。返回累计文本。
async fn open_and_collect(state: &Arc<AppState>, sid: &str, after: i64, window_ms: u64) -> String {
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(after) }),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(window_ms);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    text
}

/// 1) gap 事件被桥接补发恰好一次：回放非空（早期已落库事件）之外，一条
/// 「已广播、未落库」的 delta（seq=None，模拟 flusher 攒批滞后）必须出现
/// 恰好一次 —— 修复前为 0 次（既不在回放也不在直播）。
#[tokio::test]
async fn gap_event_bridged_once() {
    let state = make_state().await;
    let sid = "gap1";
    seed(&state, sid).await;

    // 早期事件：先落库，让回放窗口非空。
    append_event(&state, sid, "status", json!({ "k": EARLY_MARKER }), 1).await;

    // pre-subscribe 广播一条不落库的 delta（flusher 滞后）。
    let handle = make_handle(&state, sid).await;
    handle.broadcast_evt(SseEvt {
        kind: "reasoning_delta".into(),
        data: json!({ "k": GAP_MARKER }),
        ts: 2,
        seq: None,
    });

    let text = open_and_collect(&state, sid, 0, 400).await;
    let gap = text.matches(GAP_MARKER).count();
    assert_eq!(
        gap, 1,
        "未落库的 pre-subscribe 广播必须被 ring 桥接补发恰好一次，got {gap}:\n{text}"
    );
    let early = text.matches(EARLY_MARKER).count();
    assert_eq!(early, 1, "早期已落库事件仍应经回放出现一次:\n{text}");
}

/// 2) ring 与持久化并存时不得双发：同一事件既落库（带 seq）又进 ring
/// （seq=None 直播副本），流中必须恰好一次。
#[tokio::test]
async fn ring_and_persist_no_double() {
    let state = make_state().await;
    let sid = "gap2";
    seed(&state, sid).await;

    append_event(&state, sid, "status", json!({ "k": DOUBLE_MARKER }), 1).await;
    let handle = make_handle(&state, sid).await;
    handle.broadcast_evt(SseEvt {
        kind: "status".into(),
        data: json!({ "k": DOUBLE_MARKER }),
        ts: 2,
        seq: None,
    });

    let text = open_and_collect(&state, sid, 0, 400).await;
    let n = text.matches(DOUBLE_MARKER).count();
    assert_eq!(
        n, 1,
        "既落库又入 ring 的事件必须只出现一次（回放覆盖 + 指纹去重），got {n}:\n{text}"
    );
}

/// 3) 同内容多重集：广播两条内容完全相同的无 seq delta，其中仅一份落库。
/// 流中该内容总共恰好两次（回放一次 + ring 补发一次）。HashSet 版指纹只
/// 留一份时会吞错份数，是多重集修复的回归钉子。
#[tokio::test]
async fn identical_pair_multiset() {
    let state = make_state().await;
    let sid = "gap3";
    seed(&state, sid).await;

    let handle = make_handle(&state, sid).await;
    let payload = json!({ "k": PAIR_MARKER });
    // 两条完全相同的广播（仅 ts 不同，指纹只看 kind+data）。
    handle.broadcast_evt(SseEvt {
        kind: "reasoning_delta".into(),
        data: payload.clone(),
        ts: 1,
        seq: None,
    });
    handle.broadcast_evt(SseEvt {
        kind: "reasoning_delta".into(),
        data: payload.clone(),
        ts: 2,
        seq: None,
    });
    // 仅一份落库（对应其中一条的持久化副本）。
    append_event(&state, sid, "reasoning_delta", payload, 3).await;

    let text = open_and_collect(&state, sid, 0, 400).await;
    let n = text.matches(PAIR_MARKER).count();
    assert_eq!(
        n, 2,
        "两条同内容广播（其一已落库）应各出现一次：回放 1 + 桥接 1，got {n}:\n{text}"
    );
}

/// 4) subscribe 之后的直播不被 ring 桥接重复：先建立 get_events 流并消费
/// 回放帧，再 broadcast_evt 一条新事件，必须恰好一次（走直播；ring 快照
/// 在 subscribe 时已定格，不含该事件）。
#[tokio::test]
async fn subscribe_then_live_not_doubled() {
    let state = make_state().await;
    let sid = "gap4";
    seed(&state, sid).await;

    append_event(&state, sid, "status", json!({ "k": EARLY_MARKER }), 1).await;
    let handle = make_handle(&state, sid).await;

    // 先建立流（subscribe_recent 已在此刻定格 ring 快照，当前为空）。
    let resp = opencoder_web::api::get_events(
        State(state.clone()),
        Path(sid.to_string()),
        Query(opencoder_web::api::EventsQuery { after: Some(0) }),
        axum::http::HeaderMap::new(),
    )
    .await
    .into_response();
    let mut stream = resp.into_body().into_data_stream();
    let mut text = String::new();

    // 阶段一：消费回放帧，直到看到早期事件（有界等待）。
    let phase1 = std::time::Instant::now() + Duration::from_millis(300);
    while std::time::Instant::now() < phase1 && !text.contains(EARLY_MARKER) {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert!(text.contains(EARLY_MARKER), "回放帧未到达:\n{text}");

    // 阶段二：subscribe 之后广播新事件 —— 只能经直播到达一次。
    handle.broadcast_evt(SseEvt {
        kind: "status".into(),
        data: json!({ "k": LIVE_MARKER }),
        ts: 2,
        seq: None,
    });

    let deadline = std::time::Instant::now() + Duration::from_millis(400);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(Ok(bytes))) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let n = text.matches(LIVE_MARKER).count();
    assert_eq!(
        n, 1,
        "subscribe 之后的广播必须只经直播出现一次（不得被 ring 桥接双发），got {n}:\n{text}"
    );
}

/// 5) 多重集份数精确消耗（N=2 钉子）：落库两份完全相同的事件、ring 里
/// 两条完全相同的未落库广播。多重集按 2 份计数把两条 ring 副本全部过滤，
/// 流中恰好两次（均为回放）；HashSet 只留一份指纹会放行一条 ring 副本，
/// 得到三次。
#[tokio::test]
async fn multiset_consumes_exact_copies() {
    let state = make_state().await;
    let sid = "gap5";
    seed(&state, sid).await;

    let payload = json!({ "k": PAIR_MARKER });
    append_event(&state, sid, "reasoning_delta", payload.clone(), 1).await;
    append_event(&state, sid, "reasoning_delta", payload.clone(), 2).await;

    let handle = make_handle(&state, sid).await;
    handle.broadcast_evt(SseEvt {
        kind: "reasoning_delta".into(),
        data: payload.clone(),
        ts: 3,
        seq: None,
    });
    handle.broadcast_evt(SseEvt {
        kind: "reasoning_delta".into(),
        data: payload,
        ts: 4,
        seq: None,
    });

    let text = open_and_collect(&state, sid, 0, 400).await;
    let n = text.matches(PAIR_MARKER).count();
    assert_eq!(
        n, 2,
        "两份已落库 + 两条同内容 ring 副本：多重集应全部过滤 ring 副本，仅剩回放两次，got {n}:\n{text}"
    );
}
