# fix(tui): parent `>` steer uses fire_turn_cancel, not hard-abort

## 背景

当用户在 TUI 中按下 `>`（steer 按钮）且父会话正在运行、没有活跃的子
agent 时，旧代码落入 `cancel.cancel()`（hard abort），直接终止 `run_loop`。
后果：整个会话死亡，steer 从未被吸收。

会话侧基础设施（`fire_turn_cancel`、`SessionState::turn_cancel`、
`with_turn_cancel` builder）在 G2 修复中已落地并有集成测试覆盖，但 TUI
SteerSubmit handler 的行为变更未应用——缺陷在生产路径中仍然存在。

## 变更

### `crates/tui/src/steer_dispatch.rs`（新文件）
- 纯函数 `resolve(subagent_focused, running, has_children, has_pending_steer)`
  返回 `Action` 枚举（`Subagent` / `CancelChildren` / `SteerParent` /
  `StartTurn` / `Noop`），将 `>` 按钮的分发决策从 `app.rs` 内联 if-else
  链中提取为可单测的纯函数（与 `gate_compact` / `gate_clear_all` 同模式）。
- 5 个单元测试覆盖全部分支，含 G1 回归守卫：
  `running_parent_with_pending_steer_steers_not_aborts`。

### `crates/tui/src/app.rs` — SteerSubmit handler
- 声明 `turn_cancel` 句柄（`session.turn_cancel.clone()`）。
- SteerSubmit handler 改为 `match steer_dispatch::resolve(...)` —
  `SteerParent` 分支调用 `opencoder_session::fire_turn_cancel(&turn_cancel)`
  替代旧的 `cancel.cancel()`。

### `crates/tui/src/worker.rs` — `rebind_session`
- 新增 `turn_cancel: &mut SharedCancel` + `new_turn_cancel: SharedCancel`
  参数，使 `/task` 切换后 `turn_cancel` 句柄指向新会话的 token。
- 测试 `rebind_session_swaps_the_active_cancel_token` 扩展为同时验证
  `cancel` 和 `turn_cancel` 均被正确重绑定。

### `crates/tui/src/app_task.rs` — `switch_session`
- 新增 `turn_cancel: &mut SharedCancel` 参数，在 session move 进 worker 前
  clone 新会话的 `turn_cancel` token，传入 `rebind_session`。

## 测试覆盖

| 文件 | 测试名 | 断言 |
|------|--------|------|
| `steer_dispatch.rs` | `running_parent_with_pending_steer_steers_not_aborts` | 无子 agent + 有 pending steer → `SteerParent`（非 hard-abort） |
| `steer_dispatch.rs` | `running_parent_with_nothing_to_do_is_noop` | 无子 agent + 无 steer → `Noop`（非 abort） |
| `steer_dispatch.rs` | `running_parent_with_children_cancels_children` | 有子 agent → `CancelChildren` |
| `steer_dispatch.rs` | `subagent_focused_always_targets_subagent` | 子 agent 焦点优先 |
| `steer_dispatch.rs` | `idle_parent_starts_new_turn` | 空闲 → `StartTurn` |
| `worker.rs` | `rebind_session_swaps_the_active_cancel_token` | 切换后 cancel + turn_cancel 均指向新会话 |

会话侧运行时契约由既有集成测试覆盖：
`crates/session/tests/parent_turn_cancel_steer.rs`
（`fire_turn_cancel` 中断 LLM 流、`cancel` 保持完好、steer 被吸收）。

## 回归结果（rules/02-regression-gate）

- `cargo test --workspace` → **1461 passed / 0 failed / 0 ignored**
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
