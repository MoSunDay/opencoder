Commit: (working-tree, post-860831d)

# Shift+Tab 快速双击伪造 handoff：双层修复（UI 同步折叠 + worker 溯源门）

## Context

偶发路径：plan→act handoff 后 `plan_submitted` 粘性保持 `true`（`TranscriptReset` 显式保留、仅 `AgentSwitch("plan")` 事件折叠它）。在事件往返窗口内快速双击 Shift+Tab（act→plan→act）：

1. 第 1 击：`handle_switch_agent` 乐观同步翻转 `chat.agent="plan"`，但 `plan_submitted` 要等 worker 的 `AgentSwitch("plan")` 事件穿过 unbounded→bounded 双通道回到 UI select 才折叠（通道拥塞时窗口拉宽）。
2. 第 2 击落窗内：`plan_to_act=true` 且 `plan_submitted` 陈旧 `true` → handoff 分支：输入框被 `std::mem::take` 清空，发出 `UiCmd::SwitchAndStart`。
3. worker FIFO 依次执行 `SwitchAgent("plan")`（重置 `plan_input_count=0`）→ `SwitchAndStart("act")`：无溯源门直接 `plan_handoff::handoff()`——`final_plan_text` 取"最后一条非空 assistant 文本" = act 模式的最后一条回答（如"任务完成"）被包装成假计划，全部 transcript 折叠为一条合成消息，`handoff_seq/handoff_plan` 落库。
4. resume 按 `handoff_seq` 裁掉全部前史——上下文完全丢失且跨重启不可恢复，保留的"计划"也是错的。

## Change Summary

- **UI 根因修复**（`crates/tui/src/chat.rs` + `app_loop.rs`）：`AgentSwitch` 事件臂的折叠逻辑抽成 `ChatView::fold_agent_switch(&mut self, to: &str)`（事件臂复用）；`handle_switch_agent` 乐观翻转 `chat.agent` 处改为调用 `fold_agent_switch`——`plan_submitted` 在击键时刻同步折叠，第 2 击走纯 `SwitchAgent` 分支（输入不丢、不 handoff）。与 `TurnDone(plan)` 的 `pending_plan_arm` 兜底消费方向一致（防丢事件重武装）。
- **Worker 纵深防御**（`crates/tui/src/worker.rs` `SwitchAndStart`）：调 `handoff()` 前加溯源门 `sess.plan_input_count > 0`（session 侧真源：直发/queue/steer/复合命令全经 `maybe_tag_plan_prompt` 递增，仅进入 plan、handoff、resume 时重置），镜像 `control_cmd.rs` ClearContext 门。门失败降级为纯切换：不 handoff、不发 `TranscriptReset/PlanHandoff`、不写 `handoff_seq`；仍持久化 agent + `clear_skill`、仍跑空 turn 满足 UI 的 `TurnDone` 协议，并 emit `SessionEvent::Status`（"handoff skipped — no plan input this phase; context preserved"）提示降级。
- **既有测试适配**（`plan_act_handoff.rs` / `agent_switch_persist.rs`）：合法 handoff 场景补 `plan_input_count = 1` 播种（真实计划阶段必有≥1 次需求交付）；行为断言不变。

行为修正（计划内）：纯 `$skill` 输入（不计入 `plan_input_count`）后 Shift+Tab 降级为纯切换而非 handoff——原行为本就会把空 transcript/无关回答捏造假计划，降级更正确。web `DrainCmd::Handoff` 同样无门，但为显式 API 调用，本 bug 范围外，留作后续加固项。

## Validation（当次实跑）

- `cargo test -p opencoder-tui`：全绿（lib 1424+ 用例 + 23 个集成套件）。
- `cargo test -p opencoder-session`：全绿（plan_handoff / control_cmd / resume 未动，回归确认）。
- `cargo test --workspace`：184 套件全绿、0 failed。
- `cargo clippy --workspace --all-targets`：零警告。
- `cargo fmt`（本轮触碰文件）：clean。

## 测试清单（rules/01）

**新增**
- `tui app_loop_tests::switch_gate_tests::{switch_act_to_plan_collapses_stale_plan_submitted_synchronously, shift_tab_double_tap_second_strike_is_pure_switch_and_keeps_input}` —— 双击回归：第 1 击后 `plan_submitted` 同步为 false；第 2 击路由纯 `SwitchAgent`、输入保留、不发 `SwitchAndStart`、不起 turn。
- 新文件 `crates/tui/tests/handoff_provenance_gate.rs`：
  - `stale_double_tap_switch_and_start_preserves_context` —— bug 的精确 FIFO 序列（`SwitchAgent("plan")` 重置计数 + 陈旧 `SwitchAndStart("act")`）：无 `TranscriptReset/PlanHandoff`、有 Status 降级提示、`TurnDone` 协议仍完成、transcript 逐字保留、无浪费 LLM 调用、不写 `handoff_seq`、resume 保留完整 act 历史。
  - `plan_phase_input_still_hands_off` —— 合法路径回归：`plan_input_count > 0` 时照常折叠 + 持久化 resume 边界。

**适配（断言语义不变）**
- `tui tests/plan_act_handoff.rs::{switch_and_start_clears_transcript_and_feeds_only_plan_to_act, switch_and_start_appends_input_to_plan_handoff, switch_and_start_clears_skill_prompt}`
- `tui tests/agent_switch_persist.rs::switch_and_start_handoff_persists_act_mode`

**既有回归（本轮相关面）**
- `tui app_loop_tests::switch_gate_tests`（运行门 5 条）、`chat_tests::agent_switch`（fold 抽取不改事件路径语义）、`app_loop_bugfix_tests::handle_switch_agent_sets_agent_optimistically`、`plan_card_full_flow` / `resume_context_used` / `subagent_replay` 等。
