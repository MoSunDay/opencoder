Commit: pending

# 记忆维护仅限 act 模式任务

## 背景

`local-memory` 打开后，任何 Primary 主任务（包括 plan 只读问答）完成都会触发 `repo-local-memory` 维护，plan 轮次不应改写仓库记忆。

## 变化

- `runner/local_memory::eligible` 增加 `agent.kind == AgentKind::Act` 门槛：plan（只读）、command、workflow 与 subagent 会话一律不执行记忆更新。
- 入口 eligibility（`eligible_for_run`）探测输入前缀的控制命令切换目标：plan 会话中提交的 `/act 任务`、`/agent <act 类> 任务`、`/act_clear_context 任务`（plan→act 交接）按切换后的 act 判定，act 会话提交的 `/plan 问题` 不再维护；`Done` 事件的发出时机与次数契约不变。

## 影响

只影响会话结束路径的记忆维护判定，Config 与 TUI 开关不变。详情见 [本地仓库记忆](../../local-memory/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| act 任务后正常维护（既有） | `enabled_memory_uses_a_context_copy_after_main_completion` | `crates/session/src/runner/local_memory/mod.rs` |
| plan 任务不触发维护 | `plan_mode_task_does_not_update_memory` | `crates/session/src/runner/local_memory/mod.rs` |
| plan 会话 `/act 任务` 复合输入仍维护 | `compound_act_switch_from_plan_still_updates_memory` | `crates/session/src/runner/local_memory/mod.rs` |
| 控制命令切换目标的 eligibility 判定 | `eligible_for_run_keys_on_the_control_switch_target` | `crates/session/src/runner/local_memory/mod.rs` |

- 定向回归：`cargo test -p opencoder-session`（lib + 集成）全量通过。
