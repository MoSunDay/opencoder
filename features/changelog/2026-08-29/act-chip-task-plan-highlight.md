# act chip：task-plan 提交态点亮 sandbox 黄

## 背景

底部状态条左下角 `[act]` chip 与 sandbox chip 同为青色，用户要求：提交 `task-plan` skill 后 act 状态转 sandbox 黄作为「规划在效」视觉提示；提交任意非 task-plan skill 或 steer/queued 输入实际生效后回原色。只有 act 状态变色。

## 实现

- **theme**（`theme.rs`）：新增纯函数 `status_chip_fg(mode, plan_skill_active)`——`act`+plan 提交态返回 `WARN`（与 sandbox 同黄），其余原样走 `agent_chip_fg`；`agent_chip_fg`/`mode_flash_bg` 行为不变（sandbox chip 本就是黄，子代理 header、模式闪光 chip 均不在范围）。
- **skill 状态**（`skill_persist.rs`）：`PLAN_CHIP_SKILL="task-plan"` + `act_plan_highlight(Option<&str>)`（精确匹配）+ `initial_skill_state()`（从共享 skill handle 派生 body/sys_tokens/起始高亮，恢复会话持久化 `task-plan` 起始即黄）。app.rs 以净零行（799→799）接入：idle 提交 `resolve_persist` 后、`KeyAction::SetSkill` 菜单提交/清除后即时定色。
- **生效回退**（`app_loop.rs::fold_ui_events`）：`QueueConsumed`（queued 输入在 idle 边界被吸收）与 `SteerConsumed`（steer 输入在 turn 边界被提升）两个事件即「生效」时刻，将 `plan_skill_active` 置 false；app.rs 本地态经 `fold_ui_events`/`frame::render_frame`/`render::render`/`render_status` 贯穿到 chip+前导圆点（两 span 共用一个 `chip_fg`）。
- **防复活**（`app_helpers.rs::refresh_skill_mirrors`）：新增 `plan_skill_active: &mut bool` 参数，仅在 body 实际变化路径重派生；early-return（body 未变）保留调用方现值——steer 生效清掉的黄不会被 idle 镜像刷新复活。runner 消费期点亮（queued `$task-plan`）按既有镜像语义等 idle（`!running` 门）后重亮。

## 测试

- 提交态点亮与色映射：`theme::tests::status_chip_fg_act_lights_yellow_for_task_plan`（act+plan→WARN；无 plan→ACCENT；sandbox 恒 WARN；explore 不受影响）。
- 实渲染：`render::tests::status_bar::status_bar_act_chip_lights_warn_for_task_plan`（TestBackend buffer 断言 `[act]` chip cell Yellow/Cyan、前导圆点同色、布局不位移）。
- 精确匹配：`skill_persist::tests::act_plan_highlight_matches_task_plan_exactly`（`task-plans` 等近名不点亮）。
- 恢复起始态：`skill_persist::tests::initial_skill_state_derives_body_tokens_and_highlight`。
- 生效回退：`app::app_loop::tests::plan_chip_consume_tests::{queue_consumed_clears_the_plan_chip_highlight, steer_consumed_clears_the_plan_chip_highlight}`（in-memory store 夹具实折事件）。
- 防复活：`app_helpers::tests::skill_apply::refresh_skill_mirrors_derives_plan_flag_only_on_body_change`（变 task-plan→true；变其他→false；body 未变早退时 true/false 均保持）。
- 既有不降：`render_tests/chips.rs` 2 例（agent_chip_fg/mode_flash_bg 旧契约）与 status_bar/status_ctx 既有断言全绿；全部 `render_status`/`fold_ui_events` 既有调用点补新参数后逐一复跑。
- 全量回归：`cargo test --workspace --no-fail-fast` → **3689 passed / 0 failed**（248 个 test result 面，TEST-EXIT=0，shellguard 收口后提交前终验实跑）。

## 边界

- 仅 TUI 父状态条；web SPA、headless CLI 无此 chip，不在范围。
- 卡在 pending 从未被吸收的 steer/queue 行不回色（无消费事件=未生效）；被消费项自带 `$task-plan` 时先随生效回色、idle 镜像同步后重新点亮（提交语义优先）。
- steer 聚焦子代理（`subagent_input`）不改父 chip——非父会话注入面。
