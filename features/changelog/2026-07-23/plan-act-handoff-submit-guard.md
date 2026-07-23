Commit: (working-tree, pre-initial-commit)

# fix(tui): plan→act handoff 仅在 plan 模式提交过内容时触发

## 背景

plan→act 切换的 handoff 决策（清空 transcript、只保留最终计划并立即执行）原由
`app.rs` 中 `!chat.blocks.is_empty()` 判断。但 `chat.blocks` 始终包含 act 历史，
恒为非空，导致：

- 用户在 act 执行完有报告后，进 plan 模式**未提交任何内容**，切回 act
  → 误将 act 的最终输出当作"计划"执行，transcript 被错误截断。
- 仅当用户在 plan 模式**实际提交过提示词**、模型产出计划后，切 act 才应触发 handoff。

## 变更

### `crates/tui/src/chat.rs`
- `ChatView` 新增 `pub plan_submitted: bool` 字段（`#[derive(Default)]` → 默认 `false`）。
- `SessionEvent::AgentSwitch(to)` 处理器：切换**到 plan** 时重置 `plan_submitted = false`，
  确保每次进 plan 干净开始；切到 act 时不动（app.rs 事件循环在该事件到达前已读取它）。

### `crates/tui/src/app.rs`
- `KeyAction::SwitchAgent`（Shift+Tab/Alt+Tab plan→act）的条件：
  `!chat.blocks.is_empty()` → `chat.plan_submitted`，仅当 plan 模式下提交过才 handoff。
- `KeyAction::Submit` 两条 idle 提交路径（正常提交 + skill-only 提交）：
  当 `chat.agent == "plan"` 时置 `chat.plan_submitted = true`。

## 设计要点

- `plan_submitted` 是纯 TUI UI 状态，不持久化到 store / session。
- `TranscriptReset`（`replay_into_chat`）通过 `..Default::default()` 正确重置为 `false`。
- `t`+Tab chord（`SwitchAgentNoClear`）行为不变——始终 plain swap，不触发 handoff。
- `plan_submitted` 仅影响 plan→act 路径，act→plan 切换不受影响。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 默认值为 false（新会话不误触发 handoff） | `plan_submitted_defaults_false` | chat_tests.rs |
| 进 plan 模式重置为 false | `agent_switch_to_plan_resets_plan_submitted` | chat_tests.rs |
| 进 act 模式保持不变（不被清） | `agent_switch_to_act_keeps_plan_submitted` | chat_tests.rs |

- 全量回归：`cargo test --workspace` → 868 passed; 0 failed
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- 行数：改动文件均在已有基础上小幅增长（+8 / +6 / +39 行）

## Impact Surface
- 用户：进 plan 模式未提交即切回 act 时，不再误截断 transcript、不再误执行。
- 无 store / session 数据形状变化，无 prompt 契约变化。
