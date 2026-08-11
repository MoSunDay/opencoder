# 流式/后台任务的协作式关闭：rx 掉线即退、退出中止残留句柄

## Context

会话取消（cancel）或客户端断开后，三类后台工作没有及时收尾，造成资源滞留：

- **LLM 流式任务**：`run_stream_once` 主循环先 `if tx.is_closed()` 轮询再 `tokio::time::timeout(idle_timeout, stream.next())`。
  两次检查之间存在窗口——若上游静默，任务会卡在 `timeout` 整段 `idle_timeout` 才退出；
  `run_stream` / `connect_with_retry` 的退避 `sleep` 与 `send_request` 也不感知 rx 关闭，
  消费者早已 drop，任务仍会重试/睡眠数秒。
- **后台命令 supervisor**（`tools/bg.rs::handoff`）：`tokio::spawn` 出来的 detached
  supervisor 只在自身完成时清理，但它的 `JoinHandle` 没人持有；`cleanup_all()` 只
  `kill_all()` 杀进程组，从不 abort 仍在运行的 supervisor 任务。
- **TUI worker**：`app_bootstrap::finish` 用 `timeout(5s, worker)` 等待收尾，超时后只是
  让 timeout 自然结束并丢弃 future，worker 任务仍可能挂起直到被 drop。

目标是让这些任务在取消/退出边界**主动、及时**收尾，而非依赖超时自然到期。

## Change Summary

### LLM 流式任务协作式关闭
- **`crates/llm/src/client.rs`**：
  - `run_stream_once` 主循环由「先轮询 `tx.is_closed()` + `timeout(idle_timeout, stream.next())`」
    改为单个 `tokio::select! { biased; _ = tx.closed() => return Ok(()) , stream.next(), sleep(idle_timeout) }`，
    消除轮询窗口：rx 一关即返回，不再卡满 `idle_timeout`。
  - `run_stream` 的退避 `backoff_delay(attempt)` 包进 `select! { biased; tx.closed(); backoff_delay }`。
  - `connect_with_retry` 的 `send_request` 与非 `Retry-After` 退避 `sleep` 同样包进 `select! { biased; tx.closed() => return Ok(None); ... }`，
    使建立连接阶段也能在消费者取消时立即终止。
  - 所有 select 分支 `biased` 排序，优先消费关闭信号。

### 后台命令 supervisor 句柄追踪 + 清理
- **`crates/session/src/tools/bg.rs`**：
  - 新增 `task_handles()`（`OnceLock<Mutex<Vec<JoinHandle>>>`），`handoff` spawn 的
    supervisor 句柄入表。
  - `cleanup_all()` 在 `kill_all()` 之后 `drain` 全部句柄并 `abort()`，确保退出时
    残留 supervisor 任务（如仍在等进程结束）不再悬挂。

### TUI worker 超时中止
- **`crates/tui/src/app_bootstrap.rs`**：`finish` 改为持有可变 `worker`；`timeout` 超时后
  显式 `worker.abort()`，而非仅让 timeout 自然结束丢弃 future。

全部改动局限于各 crate 内部生命周期，无 trait、store 数据形状、CLI、HTTP、prompt 契约变化。

## Validation

- `cargo build --workspace` → Finished，零错误零警告。
- `cargo clippy --workspace --all-targets -- -D warnings` → Finished，零警告。
- `cargo test --workspace` → `total passed=2347 failed=0`（全二进制汇总，0 failed）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| rx drop 后流式任务在 5s 内关闭上游连接 | `stream_task_exits_promptly_after_rx_drop` | `crates/llm/src/client_tests.rs` |
| handoff 追踪 supervisor 句柄，cleanup_all drain+abort 后归零 | `handoff_tracks_supervisor_handle_for_cleanup` | `crates/session/src/tools/bg.rs` |

## Related Docs

- [agents/llm](../../agents/llm/index.md)（`run_stream` / `connect_with_retry` 段）
- [agents/session](../../agents/session/index.md)（`tools/bg.rs` 后台命令段）
