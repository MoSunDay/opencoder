# bash 工具：取消自动超时/后台交接，前台等待到自然退出 + 启动即注册

## 背景

此前 bash 工具会在命令运行超过阈值（`BASH_DEFAULT_TIMEOUT_SECS`）时自动把命令
**handoff 到 detached 后台 supervisor**：前台 future 提前返回一条
"Moved to background" 提示，supervisor 继续等待子进程并把输出流式写入临时文件，
`/ps`/`/stop` 只能观测/控制这些**已 handoff** 的进程。这套自动超时 + 交接的机制带来
若干摩擦：模型收到 handoff 提示后行为不确定、超时阈值与 runner 600s 安全网之间需要
精心对齐（`590s < 600s`）、长任务被人为截断成"后台"。

本轮改为更直接的心智模型：**bash 一律在前台运行到自然退出**，不再自我超时、不再
handoff；观测/控制改由"启动即注册"实现——`/ps` 列出**所有正在运行**的 bash（不仅是
handoff 的），`/stop` 可随时强杀其进程组。

## 设计

### 1. 前台无限等待，移除超时/handoff（`tools/bash.rs`）
- 删除全部超时常量与推导：`BASH_DEFAULT_TIMEOUT_SECS`、`BASH_WARN_BUFFER_SECS`、
  `resolve_timeout_secs()`、`handoff_message()` 及编译期断言。
- 超时+handoff 分支整块替换为 `let exit_status = child.wait().await?;`——工具 future
  直接拥有 `Child` 并 `wait()` 到子进程退出（保留 `setsid` 解除控制终端、stdout/stderr
  管道捕获、`exit code` 渲染、`max_output` 截断等既有逻辑）。
- `handoff()` 在 `tools/bg.rs` 中**保留**以供参考，但前台工具不再调用它。

### 2. 启动即注册（`tools/bash.rs` + `tools/bg.rs`）
- spawn 后立即 `bg::register(pid, pgid, session_id.clone())`，使 `/ps` 能列出、`/stop`
  能强杀**正在前台运行**的命令；命令自然退出时 `bg::unregister(pid)`。
- `unregister` 幂等：若 `/stop` 已先一步 `kill_all()`/`stop()` 移除条目，自然退出后的
  unregister 是 no-op，不会误删或 panic。
- 语义对比：此前注册只发生在超时 handoff 之后；现在注册发生在 spawn 时，覆盖面从"仅
  handoff 进程"扩大到"所有运行中的 bash"。

### 3. 免除 runner 600s 安全网（`runner/execute.rs`）
- `execute_call_with_timeout` 的超时参数类型 `Duration` → `Option<Duration>`；`execute_call`
  按 `tc.name` 决定：`bash` 传 `None`，其余工具传 `Some(DEFAULT_TOOL_TIMEOUT)`。
- `None` 时 deadline future 取 `Box::pin(std::future::pending())`（永不 resolve），即安全网
  永不对 bash 触发；取消（`await_cancel`/`await_turn_cancel`）与工具自身完成仍是唯一出口。
  这样 bash 不会被 600s 截断，但中断/`/stop` 仍可即时终止（`/stop` 直接 `kill(-pgid)`
  使 `child.wait()` 返回）。
- 文档注释同步：删除"bash self-limits to 590s（严格低于安全网）"的过时描述，改为说明
  bash 传 `None` 在前台跑到退出。

### 4. `/stop` 文案精简（`tui/src/local_cmd.rs`）
- `STOP_MESSAGE`：`"Process has been forcibly terminated."` → `"Process has been terminated."`
  （杀的是自己的子进程组，并非"强行"终止远端/他人进程，措辞更准确）。

## 改动文件

| 文件 | 改动 |
|---|---|
| `crates/session/src/tools/bash.rs` | 删超时常量/推导/handoff 分支；改 `child.wait()`；spawn 注册 + 退出反注册；新增/改写测试 |
| `crates/session/src/tools/bg.rs` | `register`/`unregister`/`stop` 已是公开 API；`handoff` 保留不再被调用；测试改用 per-pid `stop`（单元测试不再调用全局 `kill_all`） |
| `crates/session/src/runner/execute.rs` | `Option<Duration>` 超时；bash 走 `None`；`None`→`pending()`；doc 同步；测试 `Some(...)` 包装 + 新增 `none_timeout` 用例 |
| `crates/tui/src/local_cmd.rs` | `STOP_MESSAGE` 文案 + 断言 |
| `crates/session/tests/tools_contract.rs` | 用前台等待用例替换原 handoff 用例 |
| `crates/session/tests/bg_kill_all.rs`（新增） | 独立二进制中隔离覆盖 `kill_all` 全局排空 |

## 并发测试隔离说明

由于"每个 bash 启动即注册"到进程全局注册表，若单元测试中存在对全局 `kill_all()` 的调用，
并行执行时会把**别的测试**正在 `sleep` 的 bash 进程一并 SIGKILL（表现为 `[exit code: -1]`）。
为此：
- 单元测试（`tools/bg.rs`、`tools/bash.rs`）**不再调用全局 `kill_all()`**，仅操作各自 pid
  （`stop`/`unregister`/直接 `libc::kill(-pgid)`），从根本上消除跨测试互杀。
- `kill_all` 的全局排空语义移到**独立集成测试二进制** `bg_kill_all.rs`（单测、独立注册表，
  无并行竞争）。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| 长命令前台跑到结束、无 handoff 提示 | `bash_long_command_completes_without_handoff` | `tools/bash.rs`（unit） |
| 运行中已注册、结束后反注册 | `bash_registers_while_running_unregisters_after` | `tools/bash.rs`（unit） |
| schema 不再暴露 timeout | `parameters_schema_hides_timeout_from_model` | `tools/bash.rs`（unit） |
| register→list→unregister 往返 | `register_unregister_roundtrip` | `tools/bg.rs`（unit） |
| `stop(pid)` 杀进程组并移除条目 | `stop_kills_registered_process` | `tools/bg.rs`（unit） |
| 有超时：挂起工具返回 timeout 错误 | `hung_tool_returns_timeout_error` | `runner/execute.rs`（unit） |
| 快速工具不受超时影响 | `fast_tool_is_unaffected_by_timeout` | `runner/execute.rs`（unit） |
| `None` 超时永不触发安全网 | `none_timeout_never_fires_for_hung_tool` | `runner/execute.rs`（unit） |
| 长命令不交接、输出完整 | `bash_tool_runs_long_command_without_handoff` | `tests/tools_contract.rs`（integration） |
| 已注册可被 `stop` | `bash_tool_registered_and_stoppable` | `tests/tools_contract.rs`（integration） |
| `kill_all` 排空并信号进程组 | `kill_all_drains_and_signals_registered_group` | `tests/bg_kill_all.rs`（integration，隔离） |
| `/stop` 文案 | `stop_message_text` | `tui/src/local_cmd.rs`（unit） |

回归（本变更所属 session crate）：`cargo test -p opencoder-session` → 364 passed / 0 failed；`cargo clippy -p opencoder-session --all-targets -- -D warnings` → clean。`cargo test/clippy/build --workspace` 因并发代理对 tui（out-of-scope）的 mid-edit 暂未全绿，待 tui 稳定后复验；该失败非本变更引入。
