Commit: (working-tree, pre-initial-commit)

# fix(session/bash): clamp timeout < outer safety net so handoff always wins

## 根因

两个 bug 叠加导致 bash handoff 形同虚设（实测 handoff 被旁路 13 倍）：

1. **模型随意放大 timeout**：bash schema 允许模型自由传 `timeout`，模型经常
   覆盖默认 120s（历史统计 ~25% 的调用 ≥ 600s）。
2. **外层 `biased select!` 抢先**（`runner/execute.rs`）：外层安全网
   `DEFAULT_TOOL_TIMEOUT = 600s` 的 deadline arm 在 `biased` 顺序中先于 exec arm
   被 poll。当模型传 `timeout >= 600`，bash 自己的 `tokio::time::timeout` **永远
   不会先于**外层 deadline 被 poll → deadline arm 返回 `ToolOutput::err` → exec
   future 被 drop → `kill_on_drop(true)` 直接杀进程 → handoff 代码路径根本没机会
   执行。模型收到的是 `tool bash timed out after 600s`，进程被杀、输出丢失、
   没有 PID、没有后台文件路径。

## 修复

核心原则：**bash 自身的 timeout 必须严格小于外层安全网。**

### `crates/session/src/tools/bash.rs`

- 新增 `pub(crate) const BASH_MAX_TIMEOUT_SECS: u64 = 590`——硬上限。
- 新增**编译期断言** `const _: () = assert!(BASH_MAX_TIMEOUT_SECS <
  crate::runner::DEFAULT_TOOL_TIMEOUT.as_secs())`——如果有人降低
  `DEFAULT_TOOL_TIMEOUT` 到 ≤ 590，编译直接失败。
- 抽取 `fn resolve_timeout_secs(&Value) -> u64`：`unwrap_or(120).min(BASH_MAX_TIMEOUT_SECS)`。
- 更新 schema description：`"Maximum runtime in seconds before the command is
  auto-backgrounded. Default 120, hard-capped at 590. Exceeding the cap does NOT
  kill the command — it keeps running in the background with output captured to
  a file."`——引导模型不要设超高 timeout。

### `crates/session/src/runner/execute.rs`

- `DEFAULT_TOOL_TIMEOUT` 可见性 `pub(super)` → `pub(crate)`，供 bash.rs 编译期断言引用。
- 更新 doc comment：`bash caps itself at 120 s` → `bash self-limits to at most
  590 s (strictly below this guard, so its handoff path always fires before the
  safety net)`。

### `crates/session/src/runner/mod.rs`

- 新增 `pub(crate) use execute::DEFAULT_TOOL_TIMEOUT;` re-export。

## 效果

模型传 `timeout: 600`（或更高）时：bash 内部 timeout 在 590s 触发 → handoff
运行 → 命令继续在后台跑、输出引流到文件 → 返回 `Ok(ToolOutput::ok("...moved
to background..."))` → exec future 正常 resolve → 外层 600s deadline 永远不会
触发。kill_on_drop 不执行，进程不被杀。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| timeout clamp：默认值 | `timeout_clamped_below_safety_net` | `crates/session/src/tools/bash.rs` |
| timeout clamp：sub-cap 透传 | `timeout_clamped_below_safety_net` | `crates/session/src/tools/bash.rs` |
| timeout clamp：≥600 截断为 590 | `timeout_clamped_below_safety_net` | `crates/session/src/tools/bash.rs` |
| 不变量：bash cap < 安全网 | `bash_timeout_cap_is_strictly_below_safety_net` | `crates/session/src/runner/execute.rs` |
| 编译期断言：cap < DEFAULT_TOOL_TIMEOUT | `const _: () = assert!(...)` | `crates/session/src/tools/bash.rs` |
| handoff 机制（已有回归） | `bash_handoff_on_timeout` | `crates/session/src/tools/bash.rs` |
| handoff 进程存活（已有回归） | `bash_tool_hands_off_on_timeout` | `crates/session/tests/tools_contract.rs` |
| handoff 输出文件（已有回归） | `bash_tool_output_file_captures_output_on_timeout` | `crates/session/tests/tools_contract.rs` |

- 全量回归：`cargo test --workspace` → **1065 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：bash.rs 349 行、execute.rs 252 行（均 < 800）
