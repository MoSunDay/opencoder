# LLM 流式请求中途重试：chunk 错误 / 截断 / idle 停滞

## Summary

LLM 流式请求此前仅在**连接建立前**有重试（`connect_with_retry`，`MAX_ATTEMPTS=5`）。
一旦 SSE 字节开始流动，遇到三种中断就只能整轮失败、把 `Error` 交给 session 层：

1. **chunk 读取错误**（连接被重置 / 对端 RST / TLS 中断）；
2. **截断流**（流正常结束但从未出现 `finish_reason`——provider 提前断流）；
3. **idle 停滞**（连接活着、keep-alive 心跳不断，但长时间无内容 delta）。

本次在 client 层新增**中途重试**（`MAX_STREAM_ATTEMPTS=3`），覆盖以上三种中断。
重试逻辑集中在 `ChatClient` 内部，4 个消费方（runner / compaction / resume /
autopilot-verify）自动受益，无需各自实现。

核心设计：

- **两层独立预算**：`MAX_ATTEMPTS=5`（连接前）/ `MAX_STREAM_ATTEMPTS=3`（连接后）。
  中途预算刻意设低——每次重试丢弃已累积的全部状态（`text_buf` / `tools` /
  `usage` / `finished` / `decoder`）从头生成，最坏 token 成本封顶在单 turn 的 3 倍。
- **重试即重建**：检测到中断后，丢弃所有累积状态，emit `LlmEvent::Retrying
  { attempt, max }`，然后重新建立连接。**持久化的文本永远来自单个 `Completed`
  帧，绝不跨尝试拼接**——保证落库一致性。
- **预算耗尽语义**：chunk 错误 / idle → `LlmEvent::Error`；截断 → 尽力而为
  `Completed`（保留已收到的部分文本，避免丢失可用输出）。
- **`idle_timeout` 内移**：原先由各消费方用 `select!` 守卫的 idle 检测，现统一移入
  `ChatClient`（由 `config.stream_idle_timeout()` 注入），消费方删除了各自的
  `select!` idle 守卫。
- **`Retrying` 事件复用**（不新增 variant）；消费方收到 `Retrying` 时清空自己的
  delta buffer（runner 清 `reasoning_buf`，其余清 `text`）。
- 所有 6 个生产构造点改为 `ChatClient::new_with_read_timeout(...,
  config.stream_idle_timeout(), ...)`。

> 注：纯重试策略（判定 / 退避 / 分类）抽取到新文件 `crates/llm/src/retry.rs`（276
> 行），把最易出 off-by-one 的边界逻辑做成无 I/O 的纯函数，可穷尽单测。

## Changes

### `crates/llm/src/retry.rs`（新建，276 行）
- 提取纯重试策略：`MAX_ATTEMPTS`、`MAX_STREAM_ATTEMPTS`、`is_retryable_status`、
  `AttemptOutcome`、`RetryDecision`、`retry_decision`、`backoff_millis`、
  `backoff_delay`（原 `connect_with_retry` 的策略函数）。
- 新增 `StreamInterruption` 枚举（`ChunkError` / `Truncated` / `IdleTimeout`）与
  `should_retry_stream_interruption()` 分类器——三种中断均判定为可重试。

### `crates/llm/src/client.rs`
- `ChatClient` 新增 `idle_timeout: Duration` 字段。
- 新增 `run_stream` 包裹 `run_stream_once`，在中断时执行中途重试循环。
- `run_stream_once` 检测三类中断：chunk 读取错误、截断（无 `finish_reason`）、idle
  停滞；返回 `OnceError::Interrupted { reason, partial }`（携带部分 `StreamOutcome`）。
  事件级 idle 看门狗用 `Instant::now()`，在每解码出一个 SSE 帧时重置。
- 连接错误经 `OnceError::Connect` 处理（仍走原有连接前预算）。
- 新增 `new_with_read_timeout(..., idle_timeout, ...)` 构造器。

### `crates/llm/src/event.rs`
- `LlmEvent::Retrying` docblock 扩展，说明中途重试语义（消费方须丢弃累积 delta）。

### `crates/llm/src/lib.rs`
- 新增 `pub mod retry;`。

### 消费方（4 处）
- `crates/session/src/runner/llm_call.rs`：删除 idle `select!` 守卫，保留
  cancel/turn_cancel 守卫；收到 `Retrying` 时 `reasoning_buf.clear()`。
- `crates/session/src/compaction.rs`：删除 idle 守卫，`Retrying` 时 `text.clear()`。
- `crates/session/src/resume.rs`：`Retrying` 时 `text.clear()`。
- `crates/session/src/autopilot/verify.rs`：`Retrying` 时 `text.clear()`。

### 生产构造点（6 处）
- `tui/src/worker.rs`、`tui/src/model_session_switch.rs`、
  `tui/src/app_bootstrap.rs`、`tui/src/app_loop_model.rs`、`web/src/api.rs`、
  `cli/src/run.rs` 全部改为 `new_with_read_timeout(..., config.stream_idle_timeout(), ...)`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 中途重试策略：退避倍增 + 预算上限 | `backoff_millis_doubles_each_attempt` | `crates/llm/src/retry.rs` |
| HTTP 状态分类为 outcome | `attempt_outcome_classifies_status` | `crates/llm/src/retry.rs` |
| 三类中断均判定可重试 | `all_stream_interruptions_are_retryable` | `crates/llm/src/retry.rs` |
| 截断流：重试后成功，最终 Completed | `truncated_stream_retries_then_completes` | `crates/llm/tests/stream_retry.rs` |
| chunk 错误：重试后成功 | `chunk_error_retries_then_completes` | 同上 |
| idle 心跳：重试后成功 | `idle_heartbeat_retries_then_completes` | 同上 |
| 预算耗尽：emit Error 且不 Completed | `retry_exhaustion_emits_error` | 同上 |
| 消费方收到 Retrying 时清空累积状态 | `mid_stream_retry_clears_accumulated_state` | `crates/session/src/runner/llm_call.rs` |

> 迁移说明：原 `client.rs` 内联的 6 个重试策略单测随策略函数一并迁移到
> `retry.rs`（保持等价断言）；原 `stream_timeout.rs` 的
> `stalled_stream_interrupted_by_read_timeout` 因行为模型变更（停滞改为重试而非
> 立即超时）被移除，由 `stream_retry.rs` 的 4 个集成测试替代。净测试数增加。

## 全量回归

- 全量回归：`cargo test --workspace` → **1637 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`retry.rs` 276（新增 ≤400）；`stream_retry.rs` 283（新增 ≤400）；
  `client.rs` 639（迭代 ≤800）；`compaction.rs` 778（迭代 ≤800）

## 备注

- 截断流预算耗尽时发 best-effort `Completed`（保留部分文本），是有意为之——避免在
  provider 偶发提前断流时丢失可用输出。chunk 错误 / idle 耗尽则发 `Error`。
- 时间断言均为上界 sanity 守卫（`< 4s` / `< 8s`），非紧时间窗，mock SSE 服务器确定
  性，flaky 风险低。
