Commit: (working-tree, pre-initial-commit)

# `/plan + 内容` 算需求输入：延迟武装 plan→act handoff

## 背景
act 模式输入复合 `/plan <内容>`（如 `/plan fix the bug`、`/plan $review`）并提交后，
计划 turn 完成、plan 卡片展示，但按 Shift+Tab 期望「保留计划开始执行任务」时只会
**纯切换**到 act 模式（不清空 transcript、不自动开工）。

根因：`plan_submitted` 的武装依赖提交瞬间 `chat.agent == "plan"`（`note_requirement_submitted`
在 act 模式下为 no-op），且模式切换是异步落地的——runner 处理 `/plan xx` 时发出的
`SessionEvent::AgentSwitch("plan")` 会**重置** `plan_submitted = false`。于是复合 `/plan`
交付的需求永远没有武装 handoff。

## 变更

### 1. `chat_types.rs` — 新增 `ChatView::pending_plan_arm: bool`
延迟武装旗标：提交路径置位，`AgentSwitch` 事件消费。

### 2. `chat.rs` — `AgentSwitch` 处理器消费延迟武装
- `to == "plan"` 时 `plan_submitted = self.pending_plan_arm`（原重置逻辑保留，
  无 pending 时行为不变；`requirement_submitted_then_reenter_plan_resets_flag`
  等既有契约不受扰）。
- **任意** AgentSwitch 后清空 `pending_plan_arm`：陈旧旗标不可能为后续某次
  进入 plan 模式重新武装（如提交后被取消、未真正切换）。

### 3. `control_helpers.rs` — `is_compound_plan_cmd(clean)`
`plan_compound_for_submit(clean).is_some()` 的语义化别名：`/plan <内容>`（含
`$skill` token）为真，裸 `/plan`/`/act`/普通文本为假。

### 4. `app.rs` — 三条投递路径置位，Cancel 清位
- `KeyAction::Submit`（idle 直跑 + running 排队两个子路径共用一点）、`Steer`、
  `Queue` 在 `forward_skill_if_compound` 之后：当 `chat.agent != "plan"` 且
  `is_compound_plan_cmd(&clean)` 时置 `pending_plan_arm = true`。
- `KeyAction::Cancel` 清 `pending_plan_arm`：被取消的复合 `/plan` 不会到达
  runner 的 AgentSwitch，陈旧旗标必须当场丢弃。

### 5. `app_loop.rs` — TranscriptReset 保留 + TurnDone 兜底消费
压缩重建 ChatView 时与 `plan_submitted` 同步保留 `pending_plan_arm`
（计划 turn 中压缩不掉武装）。

`UiEvent::TurnDone(agent)` 兜底：`AgentSwitch` 事件经 `try_send` 转发可能被丢
（UI 通道饱和，Bug #8 族）；事件通道 FIFO 保证——TurnDone 到达时若
`pending_plan_arm` 仍未消费且权威 agent == "plan"，则切换事件必被丢弃，
就地消费武装（plan_submitted = true、清旗标），避免陈旧武装污染后续 plan 模式进入。

### 6. `keybind.rs` — 帮助文本
Shift+Tab 条目追加：「/plan + 内容 提交后算需求输入，Shift+Tab 保留计划开始执行任务」。

## 效果

act 模式 `/plan fix` 提交 → 计划 turn 生成 plan → 空闲时 Shift+Tab = 保留计划、
清空 planning transcript、`SwitchAndStart` 自动开始执行任务（与 plan 模式内
Enter 提交需求后的 Shift+Tab 同路径）。帮助文本同步说明。

## 测试覆盖

| 测试 | 文件 | 说明 |
| --- | --- | --- |
| `compound_plan_cmd_matches_plan_compound` | `control_helpers_tests/plan_compound.rs` | `/plan <内容>` 为真；裸 /plan、/act、普通文本、空串为假 |
| `compound_plan_from_act_rearms_handoff_on_plan_switch` | `chat_tests/requirement_submit.rs` | pending → AgentSwitch("plan") → plan_submitted 重新武装且旗标被消费 |
| `compound_plan_rearm_survives_transcript_reset` | `chat_tests/requirement_submit.rs` | TranscriptReset 保留武装（压缩中不丢） |
| `stale_pending_arm_consumed_on_non_plan_switch` | `chat_tests/requirement_submit.rs` | 切到非 plan 消费并丢弃旗标，之后进入 plan 不误武装 |
| `compound_plan_from_act_armed_then_shift_tab_triggers_handoff` | `app_loop_tests/mod.rs` | 完整链路：act 提交置位 → 事件武装 → idle Shift+Tab → `SwitchAndStart("act")` + running |
| `fold_turn_done_plan_consumes_stale_pending_arm` | `app_loop_tests/mod.rs` | AgentSwitch 被丢（try_send 饱和）时 TurnDone(plan) 兜底消费陈旧武装 |

## 回归

`cargo test --workspace` → 1934 passed / 0 failed（含既有 `plan_submitted` 四契约测试）；
`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
