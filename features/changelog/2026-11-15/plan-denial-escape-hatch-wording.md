# plan 模式拒绝文案去掉不存在的 `/agent act` 说法

## 问题与根因

plan 模式只读门的模型可见拒绝（`bash_guard::plan_denial`）逃生口写的是
"switch to the act agent (/agent act)"。`/agent act` 不是文档化命令
（控制命令只有 `/act`/`/plan`/`/act_clear_context`，`/agent <name>` 是带
名字的通用形式）；sandbox 时代同类文案 2026-08-30 已修为不出现
`/agent act`，但 2026-09-01 sandbox→plan 合并时候选分支的旧写法被逐字
带回 plan 文案。

## 变更

- `crates/session/src/bash_guard.rs`：`plan_denial` 两个分支（bash /
  非 bash）逃生口统一为 "To make changes, switch to the act agent."，
  不再点名任何命令；doc 注释同步。
- `crates/session/tests/bash_guard_plan_mode.rs`：模块 doc 与两处断言
  对齐——正向断言 `switch to the act agent` 存在、反向断言
  `/agent act` 不存在。
- 非目标：`/agent <name>` 解析器（`agent_skill_pools.rs` 中的
  `"/agent act"` 输入是合法解析用例）、sidecar_denial、gate 白名单均
  不变。

## 测试清单

| 类别 | 测试名 | 位置 |
|------|--------|------|
| 修正 | `denial_names_mode_forbids_retry_points_at_act`（+`/agent act` 反向断言） | bash_guard.rs（lib tests） |
| 修正 | `plan_mode_blocks_write_command`（+反向断言） | tests/bash_guard_plan_mode.rs |
| 修正 | `plan_mode_refuses_unadvertised_tool_without_executing`（+反向断言） | tests/bash_guard_plan_mode.rs |

定向验证：`cargo test -p opencoder-session --lib bash_guard` 43/43 过、
`--test bash_guard_plan_mode` 11/11 过、clippy 零警告。
