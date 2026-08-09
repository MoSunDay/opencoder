# 运行时缺陷清扫：12 处审计确认 bug 修复

## 背景

构建清洁、2248 测试全绿。代码审计发现 12 处确认的真实 bug（运行时逻辑缺陷、资源
泄漏、静默错误吞噬）。本轮集中修复，每个修复附测试。无架构变更，均为小而精准的编辑。

## 变更

### 🔴 P0 — 确认的正确性缺陷

**Bug 1 — 重复 `LlmEvent::Error` + 双重前缀**（`crates/llm/src/client.rs`）
- `run_stream` 耗尽重试后既 `tx.send(Error)` 又 `return Err`，spawn 闭包再发一条
  `Error("stream failed: stream failed: …")`。已由上一轮 sweep（Bug 4）修复，本轮
  验证 `retry_exhaustion_emits_single_non_doubled_error` 仍守护。

**Bug 2 — BroadcastStream Lagged 静默丢弃**（`crates/web/src/api.rs`）
- `GET /events` 的 `.filter_map(|r| r.ok())` 把 `Err(Lagged(n))` 变 `None` 丢弃。
  订阅者落后时事件无声消失，丢失 `done` 事件会让 UI 永久旋转。
- 修复：抽出纯函数 `map_broadcast_result`，将 `Lagged(n)` 映射为一条 `error` 事件
  `{"error":"event lag: N events dropped"}`，让客户端感知需重新同步。

**Bug 3 — tool panic 无隔离**（`crates/session/src/runner/mod.rs`）
- `FuturesUnordered` 直接 poll tool future，任一 tool `execute` 内 panic 击穿整个
  `run_loop`，在途 subagent 无清理、DB 残留 `Running`。
- 修复：每个 tool future 包 `AssertUnwindSafe(...).catch_unwind()`，panic 转为
  `is_error: true` 的 `ToolOutput`。新增 `panic_message` 辅助函数提取消息。

**Bug 4 — 压缩 Ok(None) 仍发超限请求**（`crates/session/src/runner/mod.rs`）
- `should_compact` 为真但 `compact` 返回 `Ok(None)`（无可摘要的单条超大消息）时，
  `last_err = None` 后 break，落入 LLM 调用发送超 context window 的请求触发 400。
- 修复：`Ok(None)` 时设 `last_err` 并 break，由既有错误路径发 `Error` 事件 + 返回 `Err`。

### 🟠 P1 — 资源泄漏 & 错误掩盖

**Bug 5 — Handle 泄漏**（`crates/web/src/api.rs` + `handle.rs`）
- `GET /events` 的 `or_insert_with` 创建的 handle 在 SSE 流结束后不移除，长期运行
  服务器内存无限增长。
- 修复：`SessionHandle` 增加 `subscribers: AtomicUsize` 计数；新增 `DropGuardStream`
  包装 SSE 流，drop 时（客户端断开或自然结束）触发 `release_events_subscriber`：
  仅当「本请求创建 + 无活跃 drain + 无其他订阅者」时移除 handle（所有订阅/自增在
  HandleMap 锁下进行，`prev==1` 判定对并发订阅者权威）。

**Bug 6 — subagent flusher task 泄漏**（`crates/session/src/runner/subagent.rs`）
- 强制取消 subagent 时 flusher `JoinHandle` 被 drop 但 task 未 abort。
- 修复：新增 `FlushAbortOnDrop` RAII guard 持有 handle，drop 时 `abort()`；正常完成
  路径 `take()` 出 handle 并带 30s 超时 await（取消时 guard 仍持 handle → abort）。

**Bug 7 — 存储错误被掩盖为 404**（`crates/web/src/api.rs` + `api_ops.rs`）
- `unwrap_or(None)` / `.ok().flatten()` 把 DB 错误变成 "session not found"。
- 修复：`delete_session`、`fork`、`compact`、`handoff` 改为 `match` 区分 `Ok(None)`→404
  与 `Err`→500；`post_agent`/`post_model` 的 TOCTOU 回滚捕获改为 `warn!` 记录。

**Bug 8 — 静默吞错**（`api.rs` / `handle.rs` / `api_ops.rs`）
- `let _ = store.update_session(...)` / `let _ = cmd_tx.send(...)` 丢弃错误。
- 修复：关键写（skill 持久化）传播为 500；drain 命令发送（cmd_tx）改为 `warn!` 记录。

### 🟡 P2 — 健壮性改进

**Bug 9 — re-absorb 循环不检查 queues**（`crates/session/src/runner/mod.rs`）
- `run_with_registry` 尾部 re-absorb 只检查 `has_pending_steers`，迟到的 queue 输入被
  TUI 搁置。
- 修复：循环条件改为 `has_pending_steers || has_pending_queues`。

**Bug 10 — 非法 API key 静默丢弃 Authorization 头**（`crates/llm/src/client.rs`）
- `build_header_map` 中 `HeaderValue::from_str` 失败时静默跳过 auth 头，401 报错无提示。
- 修复：改为返回 `Result<HeaderMap>`，auth 头构造失败时返回明确错误；custom 头仍容错。

**Bug 11 — bundle 递归导入无深度限制**（`crates/store/src/bundle.rs`）
- `import_bundle_inner` 递归导入 subagent 无深度上限，恶意嵌套可栈溢出。
- 修复：`depth > MAX_BUNDLE_DEPTH(32)` 时返回 `Err`。

**Bug 12 — client 远端流式 task 泄漏**（`crates/client/src/remote.rs`）
- `events()` 的 spawn task 在接收端 drop 后仅靠 keepalive 期间无清理，task + HTTP 连接
  滞留至服务端关流。
- 修复：主循环 `tokio::select!` 监听 `tx.closed()`，所有 receiver drop 时立即返回。

## 测试映射（功能 → 测试名）

| 修复 | 测试名 | 位置 |
|------|--------|------|
| Bug 1 双重前缀（守护） | `retry_exhaustion_emits_single_non_doubled_error` | `crates/llm/tests/stream_retry.rs` |
| Bug 2 Lagged → error | `lagged_is_surfaced_as_error_not_dropped` | `crates/web/tests/broadcast_lag_handling.rs` |
| Bug 2 Ok 事件原样通过 | `ok_event_passes_through_unchanged` | `crates/web/tests/broadcast_lag_handling.rs` |
| Bug 3 tool panic 隔离 | `panicking_tool_does_not_crash_run_loop` | `crates/session/tests/tool_panic_isolation.rs` |
| Bug 4 Ok(None) 超限报错 | `over_budget_with_nothing_to_compact_errors_before_llm_call` | `crates/session/tests/compact_none_over_budget.rs` |
| Bug 5 创建者离开清理 | `release_subscriber_evicts_creator_handle_when_last_and_idle` | `crates/web/src/handle.rs` (in-crate) |
| Bug 5 drain 中保留 | `release_subscriber_keeps_handle_while_draining` | `crates/web/src/handle.rs` (in-crate) |
| Bug 5 非创建者保留 | `release_subscriber_keeps_handle_for_non_creator` | `crates/web/src/handle.rs` (in-crate) |
| Bug 6 drop 时 abort | `flush_guard_aborts_task_on_drop` | `crates/session/src/runner/subagent.rs` (in-crate) |
| Bug 6 take 后正常完成 | `flush_guard_take_disarms_and_task_completes` | `crates/session/src/runner/subagent.rs` (in-crate) |
| Bug 7 存储错误→500 | `post_skill_store_error_returns_500_not_404` | `crates/web/tests/store_error_surfacing.rs` |
| Bug 7 不存在→404 | `post_skill_nonexistent_returns_404` | `crates/web/tests/store_error_surfacing.rs` |
| Bug 8 skill 持久化→500 | `post_prompt_skill_persist_error_returns_500` | `crates/web/tests/store_error_surfacing.rs` |
| Bug 9 re-absorb 检查 queue | `reabsorb_tail_picks_up_queued_input_missed_by_in_loop_poll` | `crates/session/tests/reabsorb_checks_queues.rs` |
| Bug 10 非法 key 报错 | `invalid_key_bytes_are_reported_not_silently_dropped` | `crates/llm/tests/headers.rs` |
| Bug 11 深度限制 | `deeply_nested_bundle_exceeding_max_depth_is_rejected` | `crates/store/src/bundle.rs` (in-crate) |
| Bug 12 drop→task 退出 | `dropping_receiver_prompts_stream_task_exit` | `crates/client/tests/events_drop_exits.rs` |

## 回归

- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- `cargo test --workspace` → **2287 passed / 0 failed**
- `cargo build --workspace` → 零错误
- 行数 gate：所有新增文件 ≤ 400 行；迭代中文件 ≤ 800 行（api.rs 791、client.rs 787、mod.rs 717）

## 风险与取舍

- **P0 #3（catch_unwind）**：`AssertUnwindSafe` 绕过借用检查；tool 内部状态均在
  `Arc<Mutex>` 后，panic 至多毒化 mutex，run_loop 继续以 error result 收尾。
- **P1 #5（handle 清理）**：drop 内异步清理经 `tokio::spawn` 推迟；计数自增/自减均在
  HandleMap 锁下，`prev==1` 判定对并发订阅者权威，移除 Sender 不影响无订阅者的通道。
- **P2 #10**：`build_header_map` 签名由 `HeaderMap` 改为 `Result<HeaderMap>`，更新全部
  调用点与测试。
