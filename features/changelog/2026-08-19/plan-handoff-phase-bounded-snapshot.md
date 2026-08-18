Commit: (working-tree, post-860831d)

# plan→act 交接改为阶段有界快照 + 武装移到消费时刻（杜绝偶发清空全部上下文）

## Context

Shift+Tab plan→act 偶发"清空全部 context 且不保留计划"，两个根因：

1. **计划提取无阶段边界**：`plan_handoff::handoff` 扫全 transcript 取"最后一条非空 assistant 文本"当计划。plan 阶段有提交但 turn 无产出（失败/取消/纯 skill 提交）时，提取到的是**上一 act 阶段的回答**，包装成"计划"后折叠全部 transcript 并落 `handoff_seq` 边界——resume 永久裁掉前史，不可恢复。
2. **武装时机错位**：`plan_submitted` 在 **admit 时刻**武装（steer_fire / queue_admitter / 复合 `/plan` 的 `pending_plan_arm`）。提交 ≠ 交付：滞留（永不消费）的行也会武装，Strand 场景叠加根因 1 造成"假计划交接"。

## Change Summary

- **B4 快照单一来源**（`crates/session/`）：
  - `plan_phase.rs` 新增 `plan_snapshot_update(&AgentKind, &Message) -> Option<String>`（Plan kind + Assistant + 非 synthetic + 非空文本）+ `reset_plan_phase`；`lib.rs::SessionState::record` 命中即更新 `plan_snapshot` 并 `persist_plan_phase`（记录时刻捕获，覆盖 compaction 兜底之外的全部路径）。
  - `plan_handoff.rs::handoff` **只读 `session.plan_snapshot`**，删除全 transcript 扫描；快照空 = 本阶段无计划 → 返回 None、上下文不动。`final_plan_text` 仅保留 compaction 捕获用途。
  - `control_cmd.rs`（`/act_clear_context`）同享新语义：plan 阶段无产出 → 哨兵 fresh-start 路径，不再 fabricate。
- **B5 武装移到消费时刻**（`crates/tui/`）：
  - 删除 admit 时刻武装三处：`steer_fire.rs`（steer admit）、`queue_admitter.rs::admit_running`（Tab-queue/Enter-while-running）、`handle_queue` + `app.rs` 两处复合 `/plan` 的 `pending_plan_arm`（字段整体移除）。
  - 新武装点：`app_loop.rs` 的 `UiEvent::TurnDone(plan)` 从**持久 `plan_input_count`** 权威重武装（计数在需求真正交付给 plan agent 时递增并即落库：runner 直发路径 + `record_compound` queue/steer 孪生路径；skill-only/裸命令永不计数）。天然覆盖复合 `/plan X`（QueueConsumed 先于 AgentSwitch，计数在 TurnDone 前已持久）、resume/`/task` 切换（原有计数重武装保持）。store 读失败保持现值（fail-open）。
  - `chat.rs::fold_agent_switch`：进入 plan 一律清 `plan_submitted`（开新阶段），武装只经 TurnDone 重建。Enter 空闲直发路径的即时武装保留（提交即开跑，消费必然发生）。
  - 降级语义固化：plan 阶段有提交但 turn 无产出 → Shift+Tab 不交接、上下文保留、输入框文本按普通 act prompt 重提（worker `SwitchAndStart` 的 `plan_input_count > 0` 门 + `handoff_run_prompt`，前轮已就位，本轮 B4 后 `handoff` 返回 None 使其自然生效）。
- **B6**（前轮已完成）：worker 降级路径用 `handoff_run_prompt`。
- 测试迁移：所有直接播种 messages 依赖旧扫描语义的用例（session `plan_handoff.rs` 8 例、`handoff_resume.rs`、`control_cmd.rs` 2 例、`clear_context_regression.rs`）改为播种 `plan_snapshot` 或经 `record()` 真实捕获。

## Validation（当次实跑）

- `cargo test -p opencoder-session --tests`：全绿（含新 `plan_phase_no_fabrication` 2 例）。
- `cargo test -p opencoder-tui`（lib 1440 + 集成）：全绿。
- `cargo test --workspace`：**2985 passed / 0 failed**；`cargo clippy --workspace --all-targets` 0 warning。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `session tests/plan_phase_no_fabrication.rs::failed_plan_phase_hands_off_nothing_even_with_act_history` | integration | 核心回归：plan 阶段提交失败/取消 + 前一 act 阶段回答存在 → handoff 无产出（None）、transcript 不动、无 `handoff_seq` 落库——假计划与上下文清空双双消失 |
| `session tests/plan_phase_no_fabrication.rs::recorded_plan_output_is_handed_off_from_the_snapshot` | integration | `record()` 捕获的真实计划经快照交接；调用方持久化 `clear_plan_snapshot` 后 store 镜像解除武装 |
| `session plan_handoff.rs::handoff_uses_snapshot_only_not_uncaptured_live_text` | integration | 未捕获的 live assistant 文本不得泄漏进交接（旧语义反转为契约） |
| `session plan_handoff.rs::handoff_does_not_touch_store` | integration | plan agent 下 `record()` 端到端捕获快照；handoff 只折叠内存不动 store |
| `session plan_phase.rs`（单测，前轮） | unit | `plan_snapshot_update` 过滤条件 / `reset_plan_phase` 清计数+快照 |
| `tui app_loop_tests::fold_turn_done_plan_rearms_from_persisted_counter` | unit | TurnDone(plan) 从持久计数权威重武装：count=2 → 武装（含翻正 stale-true→false 的 count=0 → 解除武装） |
| `tui app_loop_tests::fold_turn_done_act_leaves_arm_untouched` | unit | act 阶段 TurnDone 不触武装（含 stale 计数存在时） |
| `tui app_loop_tests::queue_admit_alone_does_not_arm_shift_tab` | unit | Tab-queue admit 本身不武装：Shift+Tab 纯切换、零命令、不起 turn |
| `tui steer_fire`（`keyboard_steer_in_plan_mode_does_not_arm_plan_submitted` 等 2 例） | unit | plan 模式 steer admit 不武装（武装是消费时刻）；act 模式镜像不变 |
| `tui queue_admitter`（`admit_running_success_does_not_arm_plan_handoff` 等） | unit | admit-while-running 成功/失败均不武装，失败回滚镜像+图片快照 |
| `tui chat_tests::requirement_submit.rs::entering_plan_always_collapses_the_arm` | unit | 进入 plan 清 stale 武装；act 切换保持 sticky 由 plan 入口收敛 |
| `session clear_context_regression.rs`（3 例，前轮已修） | integration | `/act_clear_context` 各路径在新快照语义下保 plan / 哨兵降级正确 |
