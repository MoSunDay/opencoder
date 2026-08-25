Commit: 48cb5e605a38a42537951be40d24dddcba334e36

# drain/steer 应用点 hard-cancel 守卫：取消后不再自动应用排队模式命令

## 背景与根因

3542e74（延迟模式命令 admission）放行运行中排队/steer 文本模式命令后，应用点在 drain/steer 边界执行时被认定为「无条件执行、无在途 turn」——该结论对 turn 竞态成立，但遗漏了**外部异步 hard cancel**（TUI Esc×2、web POST /interrupt）与 queue/steer 应用点之间的竞态：用户 plan 会话运行中 Tab 提交 `/act` 排队、随即取消，idle 边界仍会消费并应用该行 → 会话在「已取消的运行」里被切到 act，且 cancel 抑制了 Done 重同步，前端无法感知。

竞态窗口：`claim`（原子事务，无守卫，保持不变）→ `QueueConsumed`/`SteerConsumed` 事件 → `apply` 模式切换，全程子毫秒级。修复把「整个 claim+apply 批次」的暴露面收窄到「单项 emit+apply」的固有边界。

## 核查结论（为什么安全）

- 取消后无自动重跑：TUI `cancelled=true` 抑制 Done 重同步与 drain_pending 重启；web `/interrupt` 后 `should_restart_drain` 要求 `!cancelled`、admit reaper 对已取消 token abort。unpromote 保留的 pending 行只会在**下次显式提交**（每次 run 新 token）时被消费。
- `control_cmd::apply` 仅 3 处调用点：entry 初始 prompt（run 前无 cancel）、steer 循环、drain queue——两处运行时路径均被守卫覆盖。
- 守卫只挡 hard cancel（`session.cancel`），不挡 turn_cancel（submit-now 语义与既有测试不变）。
- `claim_one_queued`/`claim_steers` 的事务保持无守卫：数据丢失不变式（`claim_one_queued_completes_under_hard_cancel`）保留。
- 守卫位于 `SteerConsumed`/`QueueConsumed` 事件**之前**：TUI mirror 按 seq 在事件后永久移除行，而取消后 Done 重同步被抑制——事件后 unpromote 会造成「badge 消失但 store 行仍在」的 UI 不一致；事件前守卫与既有 `cancel_keep_pending` 行为一致。

## 新稳定契约

- **queue 路径**（`drain_one_queued`，覆盖 idle_drain + drain_mode_step）：claim 成功后、`QueueConsumed`/apply 前检查 hard cancel；命中 → `unpromote_batch([seq])` → 返回 `Empty`（run 循环顶部检查随后停止）。
- **steer 路径**（`apply_steer_batch`，自 run_loop 提取至 steer.rs）：for 循环每项最开头、`SteerConsumed` 前检查 hard cancel；命中 → `unpromote_batch(steer_prompts[idx..])`（含当前项，P1-3 批量模式）→ `Status("interrupted")` → 结束 run。
- 已在处理中的那一项（事件已发出）仍会完成应用——外部异步取消与子毫秒级处理的固有边界。
- 3542e74 语义修正：应用点**并非**无条件——文本模式命令照常边界生效，但 hard cancel 是例外守卫；无 cancel 时行为与 3542e74 完全一致。

## Validation

- Session 新增（drain_tests.rs，规则 01 强制）：
  1. `idle_drain_under_hard_cancel_keeps_queued_mode_cmd_pending`——plan 会话排队 `/act` + 预触发 hard cancel → `IdleAction::Done`、`agent.name == "plan"`、store 行 agent 仍 plan、队列行 pending（len 1）、无 AgentSwitch/QueueConsumed 事件。
  2. `drain_one_queued_under_hard_cancel_keeps_plain_prompt_pending`——排队 `"hello"` + 预触发 cancel → `Empty`、行仍 pending 且 promoted_seq 为 NULL（守卫不只挡控制命令）。
  3. `steer_batch_hard_cancel_unpromotes_remaining_mode_cmd`——run_loop 级：steer `"hello"` + `/act`，on_event 收到首个 SteerConsumed 时同步触发 cancel → 第一项照常记录，第二项命中守卫：run 返回 Ok、agent 保持 plan、无 AgentSwitch、`/act` 行 pending、含 `Status("interrupted")` 事件。
- 既有回归：`claim_one_queued_completes_under_hard_cancel`（claim 无守卫不变式）、`idle_drain_*`、`drain_one_queued_*` 系列、steer/bare_steer/compound 全量保持绿。
- 结构：steer 应用循环自 run_loop 提取至 `steer.rs::apply_steer_batch`（`SteerApplyOutcome::{Continue{recorded}, Done, Cancelled}`），逻辑逐行等价迁移；mod.rs 744 行、drain.rs 369 行、steer.rs 437 行、drain_tests.rs 371 行，均在行数门限内。

## 兼容性

- 无 schema/API/配置变化；不触碰 claim 事务的无守卫设计；turn_cancel 语义不变。
