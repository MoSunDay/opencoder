Commit: (working-tree, pre-initial-commit)

# feat(tui): queued 提交也算需求提交——Tab-queue 后 Shift+Tab plan→act 触发上下文清理

## 背景

Shift+Tab plan→act 是否清上下文由 `chat.plan_submitted` 门控
（`app_loop.rs::handle_switch_agent`）：为 true 且 idle 时走 `SwitchAndStart` handoff
（清空 plan 探索上下文、只保留最终 plan），否则纯切换保留完整 transcript。此前该 flag
**只在 Enter-Submit（idle）路径置位**（app.rs 两处内联 `if chat.agent == "plan"`）；
**Tab-queue（运行中）** 与 **Steer（运行中 Enter）** 路径只 admit 到 store，从不置位。
后果：需求若以 queue 形式交给 plan agent，Shift+Tab 只会纯切换，plan 的完整探索上下文
会原样带进 act——queued 提交被排除在"需求提交"之外，行为不一致。

本轮把 queued 提交计入需求提交：Queue admit 成功即置位，Shift+Tab plan→act 因此触发
handoff（清上下文）。时序安全：Shift+Tab 在 running 时是 no-op，且 runner 在 idle 边界
排空 queue（`runner/mod.rs` idle drain），故能切时 queued 需求通常已被 plan 消费进计划；
handoff 不清 `session_inputs`，真有未消费的 queue 行也会流入 act 的 drain，需求不会丢。
Ctrl+Shift+Tab / Ctrl+T 的无清理切换不受影响。

## 变更

### 置位统一入口（`crates/tui/src/chat.rs`）

- 新增 `ChatView::note_requirement_submitted()`：`agent == "plan"` 时置
  `plan_submitted = true`；plan 模式外（act 等）无副作用。

### 接线（`crates/tui/src/app.rs`）

- Enter-Submit 两处内联 `if chat.agent == "plan" { chat.plan_submitted = true; }`
  收敛为 `chat.note_requirement_submitted()`（行为不变）。
- **KeyAction::Queue（Tab 运行中）**：普通文本与纯 skill 两种 admit 成功路径各加
  `chat.note_requirement_submitted()`——queued 提交计入需求提交（本轮行为变更）。

### 组合契约测试（`crates/tui/src/app_loop_tests/mod.rs`）

- `queue_armed_then_shift_tab_plan_to_act_triggers_handoff`：模拟 Queue 置位 →
  空闲 Shift+Tab plan→act，断言走 `SwitchAndStart`（handoff）而非 `SwitchAgent`。

## 测试覆盖

| 功能 | 测试 | 位置 |
|---|---|---|
| plan 模式置位即武装 handoff | `requirement_submitted_in_plan_arms_handoff` | crates/tui/src/chat_tests/requirement_submit.rs |
| act 模式提交不武装 | `requirement_submitted_in_act_does_not_arm_handoff` | 同上 |
| 置位跨 plan→act 切换保留（app 先读后切） | `requirement_submitted_in_plan_then_act_switch_keeps_flag` | 同上 |
| 重进 plan 复位 | `requirement_submitted_then_reenter_plan_resets_flag` | 同上 |
| Queue 置位 → Shift+Tab → handoff 触发 | `queue_armed_then_shift_tab_plan_to_act_triggers_handoff` | crates/tui/src/app_loop_tests/mod.rs |
| 门槛既有契约（未变）：未提交=纯切换 | `switch_plan_to_act_unsubmitted_is_pure_switch` | crates/tui/src/app_loop_tests/mod.rs |

- 全量回归：`cargo test --workspace` → **1587 passed / 0 failed / 1 ignored**
  （当次实跑，三轮复跑一致；ignored 为既有 `research_smoke_bing_wikipedia`，
  需真实 Chrome/网络）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：新文件 `requirement_submit.rs` 68 ≤ 400；`chat.rs` 799、`app.rs` 798、
  `app_loop_tests/mod.rs` 716（迭代 ≤800；chat.rs 已贴上限，下次改动须先拆分）

## 回归基线注记（review 缺口回填）

迭代开始时基线未预录（review 标注的流程缺口）。重建证据：本轮新增 5 个测试全部通过
（4 requirement_submit + 1 queue_armed chaining）、diff 扫描无测试删除、当次 1587
三轮复跑一致。上一轮 review 实测 1584（彼时未含 chaining 测试），与本轮 1587 存在
+2 计数差异，如实标注待下一轮基线核对（后续迭代必须迭代开始先记基线）。

## Impact Surface

- **可感知影响**：plan 模式下用 Tab-queue 提交需求的用户，Shift+Tab 切到 act 时
  上下文被清理（只保留最终 plan），与 Enter-Submit 行为一致；此前是纯切换保留全文。
- **不影响**：Enter-Submit 行为（收敛为 helper，语义不变）、Ctrl+Shift+Tab / Ctrl+T
  无清理切换、Steer 路径（保持现状未置位——与 queue 同为需求提交，若需一致性可同法补上）、
  store 形状、session runner / web / CLI headless。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](./tui-queue-panel-scroll.md)
