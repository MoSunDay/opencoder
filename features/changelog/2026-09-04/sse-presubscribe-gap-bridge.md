Commit: (working-tree, 待提交)

# SSE pre-subscribe gap 桥接：SessionHandle 广播环形缓冲补发未落库事件

## Context

daemon 的 drain run 回调对每个事件同时 (a) `tx.send` 直播广播、(b) 经异步
event flusher 落库（`event_sink` 对 TextDelta/ReasoningDelta 攒批 512 条
/8KB 才 flush）。客户端（SPA）在 POST /prompt 之后才建立 SSE 连接：事件若
在「subscribe 之前广播」且「回放查询 `events_after` 执行时还未落库」，
既不在直播流也不在回放里，对这条连接永久丢失。既有 subscribe-first +
两级去重只覆盖「subscribe 之后广播、查询之前落库」的重叠窗口，桥接不了
上述 gap。实测 reasoning_delta 丢失导致直播态 Say 无法与 ladder 配对，
布局与 done 后快照重建不一致。

## Change Summary

- **广播环形缓冲**（`crates/web/src/handle.rs`）：`SessionHandle` 新增
  `recent: Mutex<VecDeque<SseEvt>>`（`RING_CAP=4096`，对齐
  `event_sink::CAPACITY`，只需覆盖 flusher 攒批滞后 + 订阅延迟）。
  `broadcast_evt` 在同一把锁内「先 push_back（超容量逐出队首）再
  `tx.send`」；`subscribe_recent` 在同一把锁内返回 `(rx, ring 快照)`——
  append→send 与 subscribe→snapshot 互斥于同一把锁，保证 subscribe 前
  广播必在快照、subscribe 后广播必走直播，不双发也不丢。三处会话事件
  广播点（`broadcast_persist_event`、`apply_drain_cmd` 闭包、drain run
  回调）统一改走 `broadcast_evt`；nodes 的 `NodeHub.broadcast` 不动。
- **指纹集合改多重集**（`crates/web/src/sse_dedup.rs`）：
  `SeenFingerprints` 从 `HashSet` 改 `HashMap<(kind, data), usize>` 计数
  多重集——同内容事件在窗口内出现 N 份时按份数精确消耗，HashSet 只留
  一份会放行多余副本造成双发。tier-1 seq 判定与「首个转发 done 清空
  集合」TTL 保持不变。新增 `seed_bridge_seen`（全回放窗口种子，无
  baseline 过滤）专供桥接过滤器。
- **get_events 接入桥接**（`crates/web/src/api_events.rs`）：map 锁内仅取
  handle Arc + 占订阅位，锁外 `subscribe_recent()`（避免嵌套锁序）；
  在 `persisted`/`baseline`/`seen`/`max_replay_seq` 之后，ring 条目经
  `forward_live(e, &bridge_seen, max_replay_seq)` 过滤，`iter(persisted)
  .chain(iter(bridged))` 补发。顺序安全：flusher 单通道 FIFO 且结构性
  事件先冲刷挂起 delta，未落库 ring 条目发射序必晚于全部已落库条目。
- **drain 收束清空 ring**（`handle.rs`）：flusher 排空后清空——此刻所有
  条目已落库、回放可全覆盖，避免空闲期重连（after=最新 seq）把上一
  turn 广播尾巴当新事件整体重发。
- **文件预算**：`broadcast_persist_event`/`ensure_run_error_frame` 移居
  `handle_lifecycle.rs`（经 `crate::handle` re-export 保持路径稳定），
  `handle.rs` 回到 741 行（<800 上限）。

## Impact Surface

- 仅 `crates/web`：`src/handle.rs`、`src/handle_lifecycle.rs`、
  `src/handle_tests.rs`（补 import）、`src/sse_dedup.rs`、
  `src/api_events.rs`；无对外接口变更（`SessionHandle` 新增 `recent` 字段
  与两个 pub 方法均为增量）。
- 直播路径行为不变（`seed_seen` 仍是重叠窗口种子，P0-1 历史指纹不吞
  直播事件）；桥接集合只在 subscribe 后同步消费 ring 快照，不接触直播。

## 测试清单（功能 → 测试名，`crates/web/tests/sse_presubscribe_gap.rs`）

- 未落库 gap 事件补发恰好一次 → `gap_event_bridged_once`（修复前 0 次）
- ring 与持久化并存不双发 → `ring_and_persist_no_double`
- 同内容事件按份数精确出现 → `identical_pair_multiset`
- 多重集 N 份数精确消耗（HashSet 回归钉子）→ `multiset_consumes_exact_copies`
- subscribe 后直播不被桥接重复 → `subscribe_then_live_not_doubled`

回归：`cargo test -p opencoder-web` 43 个测试二进制全绿（含
`sse_overlap_dedup`/`sse_fingerprint_ttl`/`sse_done_collision`/
`replay_fidelity`/`switch_broadcast`）；`cargo clippy -p opencoder-web
--all-targets` 零警告。

## Notes / Compatibility

- 桥接用独立 `seed_bridge_seen`（全回放窗口）而非复用 `seen`（重叠
  窗口）：ring 里 subscribe 前已落库条目 seq <= baseline，沿用 `seen`
  将无法命中指纹而与回放双发；该集合只被桥接同步消费，无 P0-1 风险。
- 已知取舍：`broadcast_persist_event`（drain 外切换/终帧）写入的 ring
  条目在下次 drain 收束前留存，空闲重连可能重发少量（个位数）幂等帧。

## Related Docs

- agents/web/index.md（SessionHandle ring / get_events 桥接 / 测试索引）
