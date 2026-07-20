# session_events 写入批量化 O(tokens)→O(turn) + 模型值合理性告警

## 背景

每 turn 的 token 流（`TextDelta`/`ReasoningDelta`）逐条 `append_event` 落库，产生 O(tokens)
次磁盘写（每次一条 INSERT + commit fsync）。长输出 turn 下磁盘 I/O 与 WAL 压力陡增，是
明显的性能瓶颈。同时 `Config::load` 对模型值无任何校验，`x/y` 这类笔误会静默走到请求层报错。

## 设计

- **只缓冲磁盘路径**：UI/SSE 实时投递路径（`on_event` 回调）完全不动——每个事件仍实时转发到
  UI channel；只有落库路径走批量。
- **粗粒度映射**：`TextDelta` 与 `ReasoningDelta` 都 coarse-map 到 `EventKind::TextDelta`，
  仅这两类被缓冲；所有其它事件类型（Status/ToolStart/ToolEnd/Done/Error/...）立即 flush。
- **turn 边界 / channel close 触发最终 flush**：丢弃所有 `EventSink` clone 并 `await` flusher
  句柄即保证剩余缓冲全部落库（无损契约）。
- **崩溃语义**：最坏丢失当前 in-flight turn 少量未 flush 的 token delta；权威 turn 文本始终
  经 per-turn `messages` append 落地。
- **批量阈值**：`DELTA_BATCH=512` 条 或 `DELTA_BYTES=8*1024` 字节，先到先 flush。

## 变更

### Part A — Store 批量写层

- `Store::append_events(&[SessionEventRecord]) -> Result<Vec<i64>>`（`crates/store/src/store.rs`）：
  trait 新方法，单事务批量 INSERT，返回 seq 数组（输入序）。`append_event` 改为默认实现委托它。
- `events::append_many`（`crates/store/src/libsql_store/events.rs`）：单事务批量 INSERT + 一次
  `SELECT seq ... ORDER BY seq DESC LIMIT N` 回填，reverse 还原为写入序（emission order）。
- `LibsqlStore` 实现 dispatch（`mod.rs`）；`bundle.rs` 批量导入改用 `append_events`。
- `StubStore`（`tui/src/app_helpers.rs`）同步实现 `append_events`。

### Part B/C — 缓冲 flusher + surface 接线

- 新模块 `crates/session/src/event_sink.rs`：`EventSink`（可 clone push 句柄，unbounded channel）、
  `spawn_event_flusher`、`run_flusher`（批量 drain，close 时无损 flush）。
- `crates/session/src/lib.rs` 导出 `event_sink` mod + `EventSink`/`spawn_event_flusher`/`run_flusher`。
- **surface 接线**（调用方在 `on_event` 闭包内 `sink.push(&ev)`，run 结束后 `drop(sink)` + `flusher.await`）：
  - TUI `worker.rs`：`Prompt`/`SwitchAndStart`/`Compact` 三 arm。
  - web `handle.rs`：`drain_to_completion` run callback。
  - session `runner.rs`：subagent 子事件 flusher（unbounded + `run_flusher`）。
  - session `resume.rs`：replay 子事件 flusher。

### Part D — 模型值合理性告警

- `warn_if_suspicious_model`（`crates/core/src/config.rs`，模块级自由函数）：模型为 `x/y` 且任一侧
  <2 字符，或无 `/` 且整体 <3 字符时 `tracing::warn`（仅记录，绝不改写）。在 `Config::load` 的
  `apply_env` 之后调用。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 批量 append + 回放保序 | `events_append_and_after_replay` | `store/tests/store_integration.rs` |
| 2000 delta 无损 + O(turn) 写 | `deltas_persisted_losslessly_with_oturn_writes` | `session/src/event_sink.rs` |
| 结构事件与 delta 交错保序 | `structural_events_interleave_in_order` | `session/src/event_sink.rs` |
| 无 store 路径 drain 不 panic | `no_store_drains_without_panic` | `session/src/event_sink.rs` |
| 端到端：MockChatClient→run→sink→store 无损 + O(turn) | `token_storm_persists_losslessly_with_oturn_writes` | `session/tests/event_sink_flusher.rs` |
| ReasoningDelta 走同一缓冲路径无损 + O(turn) | `reasoning_deltas_buffered_on_same_path_as_text` | `session/tests/event_sink_flusher.rs` |

- 全量回归：`cargo test --workspace` → 688 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 构建：`cargo build --workspace` → 零错误

## 提交

- `ac033dd` — Part A/B/C/D 主体（store 批量层 + event_sink + surface 接线 + model warn）
- `8d0a01d` — 端到端集成测试
- `b883d19` — 去掉冗余 `..Default::default()` clippy lint
- `c53f761` — 清理 `capabilities_and_tools` 预存 clippy lint（needless_update）
