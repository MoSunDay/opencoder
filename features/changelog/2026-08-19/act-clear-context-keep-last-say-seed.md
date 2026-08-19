Commit: 860831d

# /act_clear_context 永不全清：plan 快照优先，否则保留最后一份 say 作中性种子

## Context

`/act_clear_context` 的降级链原来是「plan 交接 ‖ 空白哨兵全清」。两处体验缺陷：

1. **模式切换丢 plan**：切到 plan（`/plan`）会同时清 `plan_input_count` 与 `plan_snapshot`，plan→act→plan 往返后（未提交新需求）Shift+Tab 门与 ClearContext 门都可能对真实计划失明；若叠加 compaction 折叠，快照被清后连 transcript 兜底也找不到计划 → 空白全清。
2. **act 纯历史全清**：无 plan 出处时直接落空白哨兵，最后一份 assistant 回复（往往是刚完成的工作结论）被整段丢弃，新 context 从零开始。

字面语义（已确认）：**永不全清**——优先保留 plan 快照；否则保留 transcript 最后一条非空 assistant 回复（"最后一份 say"）作为新 context 的种子；仅当 transcript 确无任何 assistant 内容（全新会话）才空白 fresh-start。

## Change Summary

- **快照生命周期拆分**（`crates/session/src/plan_phase.rs` + `lib.rs`）：`reset_plan_phase` 只重置计数器（重新武装提醒标签），`plan_snapshot` 在纯切换下存活；退役点移到 `maybe_tag_plan_prompt` 记录新需求时（与 counter 自增同处同条件，守护 ecce7b0：新需求 turn 失败后不得把旧计划当本轮产出交接）。两个调用点（`control_cmd.rs` / `tui/worker.rs`）注释同步。
- **ClearContext 保留链**（`crates/session/src/control_cmd.rs`）：门保持 `kind==Plan || counter>0 || snapshot.is_some()`；plan 交接优先；否则 `final_plan_text`（最后一份 say）→ 新增 `seed_message()` 中性前缀种子（`prior context, not a new instruction`，**不用** plan→act "执行此 plan" 指令前缀包装 act 回复）；沿用 `collect_head_images` 保留图片、`after_handoff` 落边界；仅无 assistant 内容才空白哨兵。种子不发 `PlanHandoff` 事件。
- **存储/消费三端一致**：`handoff_plan` 新标记 `<<OPENCODER_CLEAR_SEED>>` + 文本（仿 `CLEAR_CONTEXT_SENTINEL`，无 schema 变更）；`resume.rs` 据此重建 `seed_message()`；TUI `session_ui/replay.rs` 与 CLI `session_cmd.rs` 剥标记渲染保留文本（哨兵仍然整体隐藏）；`runner/mod.rs` `handoff_pending` 对种子为 true（种子要续跑），仅空白哨兵停下。
- **Shift+Tab 门**（`tui/worker.rs`）：`counter>0 || snapshot.is_some()`（快照存活后纯切换往返也保持武装），注释说明存活/退役边界；handoff 本身仍只认阶段快照 + plan-tagged transcript（no-fabrication 契约不动）。

## Validation（当次实跑）

- `cargo test -p opencoder-session -p opencoder-tui` 全绿（含并发autopilot改动落地后的全量回归，见下）。
- `cargo clippy --workspace` 0 warning。
- rule 02 全量 `cargo test --workspace` 回归通过。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `session tests/clear_context_toggle_regression.rs::toggle_twice_then_clear_keeps_and_executes_plan` | integration | plan→act→plan 两次切换快照存活（清前断言），`/act_clear_context` 走真 plan 交接：PlanHandoff + 指令执行 turn（body 含 plan 与 "Execute it now"） |
| `session tests/clear_context_toggle_regression.rs::act_history_clear_keeps_last_say_seed` | integration | act 纯历史：折叠为单条中性种子消息、`handoff_plan="<<OPENCODER_CLEAR_SEED>>task done"`、恰 1 次 LLM 调用且 body 含 last say、无裸标记/无指令前缀、无 PlanHandoff、有 TranscriptReset、执行后记录回复 |
| `session tests/clear_context_toggle_regression.rs::failed_new_requirement_retires_stale_snapshot` | integration | act→plan 纯切换快照存活；`maybe_tag_plan_prompt` 记录新需求（turn 失败）后内存与 store 镜像 `plan_snapshot` 均为 None（ecce7b0 守护） |
| `session tests/clear_context_toggle_regression.rs::compound_clear_with_seed_keeps_rest` | integration | 种子路径下 `/act_clear_context retry the build`：rest 记为真实 user prompt，单次 LLM body 同时含 last say 与 rest，不含原始命令 |
| `session tests/clear_context_toggle_regression.rs::fresh_session_clear_uses_blank_sentinel` | integration | 无任何 assistant 内容（全新会话）才空白哨兵：0 次 LLM 调用、单条 marker、`CLEAR_CONTEXT_MARKER` |
| `session tests/clear_context_toggle_regression.rs::resume_rebuilds_seed_message` | integration | 手工边界（handoff_seq + seed 标记）resume 重建单条 synthetic 种子：含 last say 与中性前缀，无裸标记/无指令前缀 |
| `session tests/clear_context_regression.rs::act_mode_clear_context_seeds_last_say_not_fabricated_plan`（原 sentinel 用例改判） | integration | act 纯历史新契约：种子续跑、653e5bd 防伪造仍成立（无指令前缀、无 PlanHandoff、裸标记不达模型） |
| `session tests/clear_context_regression.rs::apply_clear_context_act_mode_seeds_instead_of_fabricating_plan`（改判） | integration | apply() 层门语义：无 plan 出处落种子标记而非空白哨兵 |
| `session tests/control_cmd.rs::clear_context_act_mode_seeds_answer_never_plan_directive`（改判） | integration | act-tagged 回复不被误认 plan：种子标记 + 无 PlanHandoff |
| `session src/plan_phase.rs::reset_plan_phase_resets_counter_but_keeps_snapshot`（改判） | unit | 纯切换只清计数，快照存活 |
