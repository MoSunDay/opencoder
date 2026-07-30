# feat(session/core): 提升 doom-loop / tool-failure 阈值 3/5 → 20

## 背景

两条「熔断」阈值的旧默认值过于激进，会在合法长会话中**过早中断**：

1. **doom-loop 守卫**（`runner/event.rs::DOOM_THRESHOLD`）：连续空 turn 数达到阈值即打破
   run loop，防止模型陷入无产出空转。旧值 `3` ——autopilot 的 act 阶段若连续发起多次
   相同 bash 调用（如轮询），极易误触。
2. **tool-failure 守卫**（`core/tool_guard_config.rs::max_consecutive_failures`）：同名工具
   连续失败达阈值即中止当前 turn。旧值 `5` ——外部资源（网络/编译）短暂抖动时，几次重试
   即被熔断，剥夺了自行恢复的机会。

## 变更

| 阈值 | 文件 | 旧值 | 新值 |
|---|---|---|---|
| `DOOM_THRESHOLD` | `crates/session/src/runner/event.rs` | `3` | `20` |
| `default_tool_failure_threshold`（`max_consecutive_failures`） | `crates/core/src/tool_guard_config.rs` | `5` | `20` |

- 两者均提升至 `20`，给合法长任务留出足够余量。
- 守卫逻辑结构不变：doom-loop 仍在连续空 turn 时打破循环；tool-failure 仍按
  `backoff_base_ms * 2^(n-1)`（cap `backoff_max_ms`）指数退避并在达阈值时中止 turn。
- `max_consecutive_failures = 0` 仍表示禁用 tool-failure 守卫，语义不变。

## 影响

- 纯默认值调整，运行时逻辑零结构变化。
- 自定义配置（`opencode.toml` 的 `tool_guard`）不受影响；仅未显式配置时的默认值改变。
- doom-loop 守卫是常量，无运行时配置面（本次仅改默认常量）。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| act 阶段连续 20 次相同 bash 触发 doom-loop 终止（阈值=20） | `doom_loop_guard_terminates_act_phase` | `crates/session/tests/autopilot.rs`（integration） |
| 连续失败达阈值（默认 20）后中止 turn | `threshold_stops_after_max_consecutive_failures` | `crates/session/tests/tool_failure_guard.rs`（integration） |
| 达阈值时发出 error 事件 | `emits_error_event_on_threshold` | `crates/session/tests/tool_failure_guard.rs`（integration） |
| 中间成功重置失败计数（阈值内不熔断） | `success_between_failures_resets_counter` | `crates/session/tests/tool_failure_guard.rs`（integration） |
| `max_consecutive_failures = 0` 禁用守卫，无限失败不中止 | `disabled_guard_allows_unlimited_failures` | `crates/session/tests/tool_failure_guard.rs`（integration） |


## 验证

- `cargo test -p opencoder-session --test tool_failure_guard` -> 4 passed。
- `cargo test -p opencoder-session --test autopilot doom_loop_guard_terminates_act_phase` -> 1 passed。
- `cargo test --workspace --all-targets` -> 全绿，0 failed。
