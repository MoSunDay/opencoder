# fix(session): subagent 中断后不再遗留 Running 悬挂任务

## 背景

当父会话在子 agent（`task` 工具）执行期间收到中断信号（hard cancel /
turn cancel / timeout）时，旧的 `select!` 直接 drop 子 future，跳过了子
agent 的清理路径。后果：

1. DB 中 subagent 任务卡在 `Running`，永不进入终态。
2. 子 session 的 registry 条目未被清理（child_cancels /
   child_turn_cancels 中残留 stale entry）。
3. 用户继续对话（resume）时 `replay_cancelled_tasks` 同步等待这些
   `Running` 任务，导致 HTTP-400 挂起。

此外，当用户在 TUI 提交新输入时，被中断的子任务会被原样 replay（重新
拉起子 agent），而非 abandon——用户希望"向前走"，不是"恢复中断的子任务"。

## 变更

### `crates/session/src/runner/execute.rs` — 两阶段 select
- **Phase 1**：`tokio::select!` 赛跑 subagent future 与三类信号
  (`HardCancel` / `TurnCancel` / `Timeout`)。关键：用 `&mut sub`（borrow）
  而非 `sub`（move），使信号胜出时 future **不被 drop**，存活到 Phase 2。
- **Phase 2**：给予子 agent `subagent_drain`（默认 15 s，可配）宽限窗口执行
  清理路径（标记任务 Cancelled、emit SubagentEnd、prune registry）。
  - 若子 agent 在窗口内完成 → 返回其结果。
  - 若宽限超时 → 调用 `force_cancel_subagent` 强制终态。
- **turn-cancel 显式传播**：hard cancel 通过 `child_token()` 已级联，但
  turn-level cancel 使用独立 token（非级联），需 `fire_child_turn_cancel`
  显式触发，使子 agent 能及时 drain 而非等待 LLM turn 结束。

### `crates/session/src/runner/execute.rs` — `force_cancel_subagent`
- 新增私有函数（:205），在宽限窗口超时时：将 DB 任务标记 `Cancelled`，
  prune `child_cancels` / `child_turn_cancels` 中的 stale entry，emit 终态
  `SubagentEnd` 使 UI 清除 subagent 面板。复刻子 agent 正常清理路径的
  副作用，确保无论子 agent 是否 wedge，终态一致。

### `crates/session/src/lib.rs` — `fire_child_turn_cancel`
- 新增 `pub fn`，按 call_id 从 `child_turn_cancels` map 中查找子 token 并
  cancel。与 `fire_turn_cancel`（全局）互补——后者是广播，前者精准定位
  单个子 agent。


### `crates/session/src/runner/subagent.rs` — steer vs hard-abort 区分
- `run_subagent` 检测到子 token 被取消时，新增 `parent_aborted` 判断：
  **父 cancel 是否也被取消**。
- **父未取消（steer）**：仅子 token 被取消（TUI `>` / web steer），将任务
  标记终态 `Failed`（`complete_subagent_task`），emit `SubagentEnd`，
  返回真实 `ToolOutput::err`——transcript 良构，任务不再被 replay。
- **父已取消（hard-abort）**：父 cancel 级联到子 token，保持旧行为——
  标记 `Cancelled`，留 tool_use dangling（无 tool_result），`run_loop`
  跳过记录，子 agent 可在下个 turn 被 replay。
- 区分依据：`child.cancel` 是父 cancel 的 `child_token()`，steer 只取消
  子 token，hard-abort 取消父 token（子也观测到）。

### `crates/session/src/resume.rs` — abandon 路径
- `replay_cancelled_tasks` 签名新增 `has_new_input: bool` 参数。
- 当 `has_new_input` 为真，**或**存在 pending steers / pending queue 时，
  调用 `abandon_cancelled_tasks`：回填终态 tool_result 保持 transcript
  良构，将任务标记 `Failed`（非 `Running`），不再 replay。
- 三种信号触发 abandon：①TUI 直接提交 user_text；②web 层 admit 到
  store 后 drain（steer = 用户显式重定向）；③queue 中有待消费的 prompt。

### `crates/session/src/runner/mod.rs`
- `run_with_registry` 传递 `has_new_input` 给 `replay_cancelled_tasks`。

### `crates/core/src/config.rs`
- 新增 `subagent_drain_secs: Option<u64>` 配置字段（默认 15 s）。
- 新增 `subagent_drain()` 方法返回 `Duration`。
- 2 个单元测试：默认 15 s、可配为 5 s。

## 测试覆盖

| 文件 | 测试名 | 断言 |
|------|--------|------|
| `tests/hard_abort_subagent.rs` | `hard_abort_during_subagent_marks_task_cancelled` | hard cancel 期间子任务进入 `Cancelled` 终态（`subagent_drain_secs: Some(2)` 强制快速 force-cancel） |
| `tests/hard_abort_subagent.rs` | `continue_after_hard_abort_does_not_hang` | hard abort 后继续对话不挂起（abandon 路径生效，replay 不再等待已终态任务） |
| `tests/parent_steer_terminal.rs` | `parent_steer_makes_subagent_task_terminal_failed` | steer → 子任务终态 `Failed`（abandon_cancelled_tasks 标记 Failed） |
| `tests/parent_steer_terminal.rs` | `continue_after_parent_steer_does_not_replay` | steer 后继续不 replay 已 abandon 的子任务 |
| `tests/parent_turn_cancel_steer.rs` | `turn_cancel_interrupts_llm_without_hard_abort` | turn-cancel 中断 LLM 流，不经 hard-abort 路径 |
| `tests/resume_replay.rs` | (8 tests) | replay / abandon / backfill 全路径回归 |

### 既有单元测试

| crate | 测试 | 说明 |
|-------|------|------|
| `core` | `subagent_drain_defaults_to_15s` | 默认宽限窗口 15 s |
| `core` | `subagent_drain_is_configurable` | `subagent_drain_secs: Some(5)` 生效 |
| `session` (lib) | `fire_child_turn_cancel` 系列 | 按 call_id 精准 cancel 子 token |

## 回归结果（rules/02-regression-gate）

- `cargo test -p opencoder-session` → **387 passed / 0 failed / 0 ignored**
- `cargo test -p opencoder-core` → **128 passed / 0 failed / 0 ignored**
- `cargo clippy -p opencoder-session --all-targets -- -D warnings` → 零警告
- `cargo clippy -p opencoder-core --all-targets -- -D warnings` → 零警告

> 注：working tree 存在无关的后台改动（TUI keybind / install-tools /
> clip-probe / theme 等 ~33 文件），上述数字为本次修复验证窗口的实跑结果。
> 本 changelog 仅覆盖 subagent hard-abort 修复涉及的文件。
