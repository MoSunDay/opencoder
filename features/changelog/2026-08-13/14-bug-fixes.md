Commit: (working-tree, pre-initial-commit)

# 14 项缺陷修复：编译阻断、用户可见 Bug、健壮性与并发问题

## 背景
对 OpenCoder 全 workspace 进行系统性 bug 审计，覆盖 8 个 crate。修复范围从编译阻断（McpServerConfig 未导出）到并发安全（drop 顺序、blocking_lock 回退、TOCTOU 窗口）再到数值溢出保护。

## 变更

### 编译阻断 / 导出修复
- **`crates/core/src/lib.rs`**：Bug 0 — `McpServerConfig` 加入 `pub use config::{...}` 导出，修复 TUI 引用编译失败。

### 会话运行时正确性
- **`crates/session/src/runner/steer.rs`**：Bug 1 — `drain_mode_step` 的 `needs_llm` 匹配 `Some(Role::Tool) => true`。此前 drain 模式执行工具调用后，trailing Role::Tool 消息导致错误 Idle，工具结果滞留未答复。新增 `drain_mode_step_proceeds_when_transcript_ends_with_tool_result` 和 `drain_mode_step_idles_when_transcript_ends_with_assistant` 测试。
- **`crates/session/src/runner/mod.rs`**：Bug 5 — `tokio::time::sleep(max_delay)` 包裹 `tokio::select!` + `await_cancel(session)`，使工具执行速率延迟可被取消中断。
- **`crates/session/src/runner/mod.rs`**：Bug 13 — turn-cancel 路径（含 late-cancel）均执行 `doom.clear()` + `tool_failures.clear()` + `bash_timeout_first = None`，防止取消后残留 doom-loop 签名误触发。
- **`crates/session/src/runner/mod.rs`**：Bug 14 — `emit()` 中 mutex 中毒时 `tracing::warn!` 记录并丢弃事件，而非静默。

### Web API 健壮性
- **`crates/web/src/api.rs`**：Bug 6 — `Config::save` 移至 TOCTOU drain re-check 之后；保存失败时回滚 session meta，拒绝请求不产生全局副作用。
- **`crates/web/src/handle.rs`**：Bug 7 — drain 结束时 `drop(guard)` 前置于 `flusher.await`，确保 draining 标志尽早清除。
- **`crates/web/src/handle.rs`**：Bug 8 — `release_events_subscriber` 添加 `blocking_lock()` 回退分支，在无 tokio runtime（如 Drop 上下文）时仍执行订阅者递减。

### 存储 / 持久化
- **`crates/store/src/libsql_store/inputs.rs`**：Bug 10 — 事务改为 `BEGIN IMMEDIATE`，减少 SQLite 写锁竞争。
- **`crates/store/src/libsql_store/schema.rs`**：Bug 11 — `set_version` 的 DELETE+INSERT 包裹在 `run_tx` 事务中，防止崩溃间残留空 `schema_version`。

### LLM 客户端
- **`crates/llm/src/client.rs`**：Bug 4 — `parse_usage` 的 total 计算改用 `input_tokens.saturating_add(output_tokens)`，防止 u64 溢出回绕。新增 `parse_usage_total_saturates_on_overflow` 测试。

### CLI
- **`crates/cli/src/lib.rs`**：Bug 12 — `--session` / `--continue` 在 `Cli`（全局参数）和 `Client` 上添加 `conflicts_with`。更新 `cli_parse.rs` 测试验证互斥拒绝。

### SSE 客户端
- **`crates/client/src/sse.rs`**：Bug 3 — `push` 方法添加 leading invalid UTF-8 字节剥离循环，防止解码卡死。新增 `strips_leading_invalid_utf8_and_advances` 测试。

### TUI
- **`crates/tui/src/control_helpers.rs`**：新增 `is_pure_control_cmd`（`#[allow(dead_code)]`）支持纯控制命令检测。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Bug 1: drain_mode trailing Role::Tool → Proceed | `drain_mode_step_proceeds_when_transcript_ends_with_tool_result` | session/src/runner/steer.rs |
| Bug 1: drain_mode trailing Assistant → Idle | `drain_mode_step_idles_when_transcript_ends_with_assistant` | session/src/runner/steer.rs |
| Bug 3: invalid UTF-8 剥离后解码推进 | `strips_leading_invalid_utf8_and_advances` | client/src/sse.rs |
| Bug 4: total_tokens 溢出饱和 | `parse_usage_total_saturates_on_overflow` | llm/src/client_tests.rs |
| Bug 12: --session/--continue 互斥 | cli_parse 集成测试（27 passed） | cli/tests/cli_parse.rs |
| Bug 11: set_version 事务原子替换单行 | `set_version_replaces_single_row_atomically` | store/src/libsql_store/schema.rs |
| Bug 13: turn-cancel 清零 doom 签名（差分回归） | `turn_cancel_clears_doom_signatures` | session/tests/doom_clear_on_cancel.rs |

- 全量回归：core 167 / store 88（+1 Bug 11）/ llm 119 / client 9 / web 81 / cli 91 / session(runner::) 48 + doom_clear_on_cancel 1（+1 Bug 13）/ tui(lib) 1238 → 共 1842 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished

## Impact Surface
- **用户可见**：`--session` 与 `--continue` 互斥；drain 模式不再丢失工具结果；total_tokens 不再溢出回绕
- **不影响**：Store trait 接口、ChatStream trait、session 主循环语义

## Related Docs
- [agents/session](../../agents/session/index.md) — drain 语义、doom-loop 守卫
- [agents/web](../../agents/web/index.md) — SSE 会话管理
