Commit: (working-tree, task-plan 中断保活 + 激活期 build subagent 全表面隐藏)

# task-plan 两阶段收敛：异常中断保活 skill + 激活期隐藏 build subagent

## 问题与根因（两段）

1. **激活期仍广告 build**：task-plan 激活（act chip 黄色）期间，系统提示词、task 工具 schema、token 估算、未知 subagent_type 错误文案四个表面都照常广告 build（全工具）subagent——计划-only 的 turn 不应广告实现委派，与 sandbox 只读的既有剥离先例同构但缺失。
2. **异常中断清掉未交付的计划**：`skill_lifecycle::run_loop_one_shot` 在任何 run 结束（Done / Error / run 级 cancel）无条件 one-shot 清除。TUI Esc 走 run 级 cancel token（`app_loop_actions.rs`），LLM 失败走 `Err`——都属异常中断，但 task-plan 一样被清（内存 + `sessions.skill` 行）。再次启动未完成任务时 resume 恢复不到 skill：act 变绿、build subagent 重新出现，与"计划未交付"直接矛盾。

## 变更

### 阶段 1：task-plan 激活期 build 全表面隐藏
- **core 抽出共享剥离**（`agent.rs`）：`BUILD_DELEGATION_CLAUSE` 常量 + `strip_build_delegation`，sandbox 剥离与 task-plan 剥离共用同一份子句措辞（措辞只活一处）。
- **session 统一镜像**（`tools/mod.rs::hide_build_subagent`）：`sandbox || task_plan_active(skill)` 一个判定喂给 `base_system`（按 skill body 剥离）、`schema_for`（task 工具 schema 不再列 build）、token 估算器、未知 subagent_type 错误文案四处调用点（`runner/llm_call.rs`、`runner/subagent.rs`、`tools/task.rs`、`prompt.rs`）。

### 阶段 2：异常中断保活 task-plan（`skill_lifecycle.rs`）
- 新增 `abort_keeps_skill(session, errored)`：run 以 `Err` 或 run 级 cancel token 结束**且**激活的是 task-plan（`latent::task_plan_active`）时，跳过 run-end 清除。`run_loop_one_shot` 与 `runner/mod.rs` 控制命令失败路径（`errored=true`）都走这个判断。
- 契约依据：task-plan 的交付 = 计划落盘（skill 消耗）；被中断的计划从未交付，keep 到下一次正常完成。
- **不变的部分**：正常 Done 仍清除（act 变绿、build 回归——压缩后系统提示词按调用重建，自然重新出现 build）；非 task-plan skill 维持严格 one-shot（`Err`/cancel 照清）；resume 本就从 `sessions.skill` 行恢复（含高亮回填），行保住 resume 即亮黄。

## 测试清单

新增回归测试：

| 阶段 | 测试 | 位置 |
|------|------|------|
| 1+2 | `task_plan_act_request_hides_build_on_every_surface`（提示词/schema/估算/错误文案四表面镜像断言） | tests/task_plan_build_strip.rs（新文件） |
| 1 | `plain_act_request_still_advertises_build`（无 task-plan 不误伤） | 同上 |
| 1 | `task_plan_unknown_subagent_type_error_omits_build` | 同上 |
| 2 | `aborted_run_keeps_task_plan_completed_run_clears_it`（中断保留→继续运行无 build→完成清除→后续运行 build 回归，全链路） | 同上 |
| 2 | `aborted_task_plan_run_keeps_skill_in_memory_and_store`（内存 + store 行双保活） | skill_lifecycle.rs inline |
| 2 | `completed_task_plan_run_still_clears`（Done 契约不回退） | 同上 |
| 2 | `aborted_non_task_plan_skill_still_clears`（非 task-plan 异常照清） | 同上 |

全量回归（`cargo test --workspace --no-fail-fast`）：**3778 passed / 0 failed，245 个测试二进制全数 ok，EXIT:0**。其中 skill_lifecycle 11 例、task_plan_build_strip 4 例单独复核通过。注意：本轮工作树同时包含另一并行任务（store/tui 的 subagent 状态功能，`task_row.rs`/`list_activity_order.rs`/`session_switch_restore.rs` 等）的未提交变更，gate 测的是合并后工作树；该任务的 changelog 条目由其自行补录。

## 非目标

- 不改 Esc/Web stop 的取消机制本身；不改 one-shot 契约对非 task-plan skill 的语义。
- 真机验收路径（TUI `$task-plan` → Esc → chip 保持黄 + `/session show` skill 行在 → 交付 → 变绿）待人工走查。
