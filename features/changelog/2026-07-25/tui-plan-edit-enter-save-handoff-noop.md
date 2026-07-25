Commit: (working-tree, pre-initial-commit)

# refactor(tui): plan-edit 改 Enter 保存 + 全屏编辑；plan→act 运行中 Shift+Tab 改为 no-op

## 背景

接 `2026-07-25/tui-plan-edit-mode.md` 引入的 plan-edit 模态编辑器与 plan→act handoff，
本次针对两个 UX 问题做行为收紧：

### plan→act 切换的延迟 handoff（pending_handoff）被移除
此前当 plan turn 正在运行时按 Shift+Tab（plan→act），切换不立即生效，而是把
待切换意图（含输入框文本）存入 `pending_handoff`，延迟到 `TurnDone` 再自动触发
（原 P0 竞态修复）。但用户要求：**运行中按 Shift+Tab 应直接无效（no-op）**，
不存在延迟切换——避免「我以为没切，结果一会儿又自动切走」的意外。

### plan-edit 退出键与快捷键不一致
- 原 plan-edit 用 `Esc`（Normal 模式）/ `Ctrl+C` 保存退出，`Enter` 插入换行；
  用户希望改用 `Enter` 保存（更直观），保存后回到回显 + 输入区。
- plan-edit 缺少主输入框已有的 readline 快捷键（Ctrl+A/E/W），体验割裂。
- plan-edit 仅占小条 composer 区域，长计划看不全。

## 变更

### 1. 移除 `pending_handoff` 延迟 handoff 机制

- **`crates/tui/src/app_loop.rs::handle_switch_agent`**：删除 `pending_handoff`
  参数。`plan_to_act && plan_submitted` 时，若 `*running` 直接 flash
  `↻ plan running…` 并提前 `return SwitchOutcome::Proceed`（不消费输入、不发命令）；
  空闲时立即 handoff 的逻辑不变。
- **`crates/tui/src/app_loop.rs::fold_ui_events`**：删除 `pending_handoff` 参数；
  `TurnDone` 的 else 分支简化为仅 `*running = false`（删除消费 pending 的代码块）。
- **`crates/tui/src/app_loop.rs::render_frame`**：删除 `pending_handoff` 参数；
  mode_flash 渲染仅保留 `flash_visible` 判断。
- **`crates/tui/src/app.rs`**：删除 `pending_handoff` 声明及 6 处引用
  （render_frame / handle_switch_agent / fold_ui_events 调用实参 + SwitchAgentNoClear /
  Cancel 两处的 `= None` 重置）。
- `grep -rn pending_handoff crates/tui/src/` → **0 处**（彻底移除）。

### 2. plan-edit：Enter 保存 + readline 快捷键 + 全屏编辑

- **`crates/tui/src/plan_edit.rs::handle_plan_edit_key`**：
  - `Enter` 现在直接 `return PlanEditAction::Exit`（双模均保存退出）。
  - 新增 `Ctrl+A`（光标到首）/ `Ctrl+E`（光标到尾）/ `Ctrl+W`
    （`composer::delete_word_back` 删前一个词），复用主输入框同一套纯函数。
  - `Ctrl+C` 退出（不变）、`Esc` Insert→Normal、Normal 下 `Esc` 退出（不变）。
  - 从 `handle_insert` 移除原 `Enter` 插入换行的 arm。
- **`crates/tui/src/app_loop.rs::enter_plan_edit`**：flash 文案
  `esc to save` → `enter to save`。
- **`crates/tui/src/render.rs::render`**：当 `plan_mode.is_some()`（编辑器激活）时，
  body / queue / skill 区域塌缩为 `Constraint::Length(0)`，`render_body` 跳过，
  composer 占满除状态栏外的整屏高度——整段计划可视可编辑。保存退出后自动恢复
  正常回显 + 输入区布局。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行中 plan→act Shift+Tab 不发命令/不改状态/flash | `switch_plan_to_act_while_running_is_noop` | `app_loop_tests.rs` |
| 空闲 plan→act 立即 handoff | `switch_plan_to_act_while_idle_triggers_handoff` | `app_loop_tests.rs` |
| 未提交计划为纯切换 | `switch_plan_to_act_unsubmitted_is_pure_switch` | `app_loop_tests.rs` |
| TurnDone 后 running 翻 false | `fold_done_clears_queue_items` 等 | `app_loop_tests.rs` |
| Enter 保存退出 | `enter_saves_and_exits` | `plan_edit.rs` |
| Ctrl+A 光标到首 | `ctrl_a_moves_cursor_to_start` | `plan_edit.rs` |
| Ctrl+E 光标到尾 | `ctrl_e_moves_cursor_to_end` | `plan_edit.rs` |
| Ctrl+W 删前一词 | `ctrl_w_deletes_word_back` | `plan_edit.rs` |
| Ctrl+C/Esc 退出（回归） | `ctrl_c_exits_*` / `esc_in_normal_exits` | `plan_edit.rs` |

- 删除 4 个测旧 pending 机制的测试：`switch_plan_to_act_while_running_defers_handoff`、
  `switch_non_plan_to_act_clears_pending`、`fold_turndone_with_pending_triggers_handoff`、
  `fold_turndone_cancelled_blocks_handoff`。

- 全量回归：`cargo test --workspace` → **1009 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **0 警告 / 0 错误**
- 构建：`cargo build --workspace` → **0 错误**
- 行数：`plan_edit.rs`=348、`app_loop.rs`=769、`render.rs`=795、`app.rs`=793，均 ≤800。

## 风险与对齐

- **移除 P0 竞态修复的影响**：原 `pending_handoff` 是为避免运行中按 Shift+Tab 的竞态；
  移除后运行中按 Shift+Tab 完全无效——即用户明确要求的行为。`Ctrl+Shift+Tab` /
  `Ctrl+U`（`SwitchAgentNoClear`，纯模式切换不清 transcript）不受影响，仍可在
  运行中切换。`Alt+Tab` 与 Shift+Tab 行为一致（运行中 no-op）。
- **Enter 语义变更**：原 Insert 下 Enter 插入换行被移除；plan 是多行文本时改用
  显式换行（粘帖或 Normal 模式编辑），或未来按需再加专属换行键。
- **纯函数式**：无新增 class；`PlanEdit` 仍为纯数据结构，行为由自由函数驱动。
- **范围**：仅触及 `crates/tui`，未改 session/web/store/llm/core。工作区其它脏文件
  （image_render/bg/model_switch 等）属其它特性，提交时按各自 changelog 排除。
