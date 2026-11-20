# Web/Store：SSE 回放改用有界最新窗口，防首连整段载入历史

一句话：`GET /api/executions/:id/events` 与 `GET /api/nodes/tasks/:tid/events`
的持久化回放从"整段 `events_after` 无上限读取"改为新契约
`events_tail`（最新窗口，≤1000 条 / 2 MiB 载荷预算），首连 `after=0`
不再把整会话历史克隆进每条连接的内存，消除长会话 × 多标签页 / 重连风暴
下的服务端内存尖峰。

## 背景与缺陷

两个 SSE 端点的回放都直接 `store.events_after(&sid, after)`——该查询没有
LIMIT，返回 `(after, head]` 区间内的**全部**持久化事件并在内存中拼成
`Vec<SseEvt>`：

- `crates/web/src/api_events.rs`（主 `/events`）；
- `crates/web/src/sse_nodes.rs`（节点任务 `/events`）。

长会话首次连接（`after=0`）会把整段历史载入内存；多个浏览器标签页或断线
重连风暴下，每条连接各克隆一份完整历史，服务端 RSS 出现尖峰甚至 OOM。
存储层早已有带行的+载荷双预算的 `events_page`（TUI transcript 翻页在用），
但这两处回放没有走有界读取。

客户端语义上这本来就是可接受的截断：SPA（`crates/web/spa/src/sse.js`）
把重连回放视为"错过的窗口的有界尾部……截断的尾部最终会收敛"（客户端侧
另有 REPLAY_CAP_FRAMES=400）；TUI 经 `/events-page` 分页取旧历史、以
cursor≈head 订阅。因此服务端回放"最新有界窗口"与既有约定一致，不需要
新增 SSE 帧类型。

## 变更清单

- `crates/store/src/store/contract/part_1.rs` — `Store` 契约在 `events_page`
  之后新增带默认实现的 `events_tail(session_id, after_seq, max_rows,
  payload_budget) -> SessionEventPage`：返回最近 `max_rows` 条（升序），
  `more` 表示区间超出预算。
- `crates/store/src/turso_store/impl_methods/part_2.rs` — `events_tail`
  委托（镜像 `events_page`，`db.read` + `events::tail`）。
- `crates/store/src/turso_store/conversation/events.rs` — 新增
  `events::tail()`：与 `page()` 相同的两阶段（元数据+载荷）读取与逐行
  预算规则（`EVENT_OVERHEAD_BYTES`、首行超限占位
  `event_payload_exceeds_page_budget` / `read_via=event_payload`、预算耗尽
  即停），但按 `ORDER BY seq DESC LIMIT max_rows+1` 从头端倒序取，最后
  反转回升序；`more = 取到 max_rows+1 行或预算提前截断`。
- `crates/web/src/sse_replay.rs`（新增）— `REPLAY_MAX_EVENTS=1000`、
  `REPLAY_BYTE_BUDGET=2 MiB` 与 `bounded_replay(store, session, after)`
  （查询失败按旧口径映射为空窗口，仅告警）；在
  `crates/web/src/lib.rs` 声明模块。
- `crates/web/src/api_events.rs` — 回放块改用
  `sse_replay::bounded_replay`，其余逻辑（baseline 先行、
  `subscribe_recent`、`seed_seen`、`max_replay_seq`、ring 桥接、直播链、
  DropGuard）逐字未动；文档注释说明回放为最新有界窗口（≤1000 条 /
  2 MiB），旧历史走分页 `/events-page`，截断可由"首条回放 seq > after+1"
  观测。
- `crates/web/src/sse_nodes.rs` — 同样替换为 `bounded_replay`（保留
  `persisted_to_evt` 映射）。

不涉及 `bundle.rs`（一次性导入）与 worker 查询路径（已是分页）；SSE 帧
类型与去重逻辑不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 界内 tail = 全量升序且 `more=false` | `tail_returns_full_range_when_under_bounds` | `crates/store/tests/contracts/bounded_pagination.rs` |
| 超界只留最新窗口（升序）且 `more=true` | `tail_keeps_newest_window_when_range_exceeds_bounds` | 同上 |
| after 游标只回放更新事件，过头为空 | `tail_respects_after_cursor` | 同上 |
| 载荷预算截断 + 单行超限占位不 panic | `tail_respects_payload_budget` | 同上 |
| 首连 `after=0` 回放 ≤1000 帧且为最新窗口（首帧 seq=total-999、末帧=head、升序） | `first_connect_replays_bounded_newest_window` | `crates/web/tests/sse_bounded_replay.rs`（新增） |

## 实测

```text
cargo clippy --no-deps -p opencoder-store -p opencoder-web --all-targets -- -D warnings
                                          # Finished，零警告
                                          #（--no-deps 因 dag-runtime 存量
                                          #  needless_range_loop 与本改动无关，
                                          #  stash 验证 HEAD 上即失败）
cargo test -p opencoder-store --test bounded_pagination   # 8 passed / 0 failed
cargo test -p opencoder-web --test sse_bounded_replay     # 1 passed / 0 failed (0.32s)
cargo test -p opencoder-web --test replay_fidelity        # 1 passed / 0 failed
cargo test -p opencoder-web --test sse_done_collision     # 1 passed / 0 failed
```

- 触及文件行数：新增 40 / 137 行（≤400）；迭代文件最大
  `bounded_pagination.rs` 548 行（≤800）。
- **未运行全量测试**：未运行 `cargo test --workspace`、SPA 全量测试或
  全站 UI 验收；仅运行上述定向用例。

## 回滚

- 直接 revert 本次提交；无 schema、协议、配置或依赖迁移
  （`events_tail` 为带默认实现的契约扩展，回滚即随代码消失）。
- 回滚后首连回放重新整段载入历史，长会话多连接下内存尖峰复现。
- 回滚时删除本 changelog。
