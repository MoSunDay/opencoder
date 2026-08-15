Commit: bd3c980275fd79dbd23173a88fdd6f4806f72bfd

# plan question 多题导航、自定义追加与光标修复

## 背景
plan agent 已能在一轮发出多个 question tool call，但 TUI 仍逐题弹出，用户不能在提交前切题检查；自定义文本还会覆盖预设答案，且输入光标没有按笔图标的实际终端宽度定位。

## 变更
- 同轮问题聚合为一个多题表单：`←/→` 切题、`↑/↓` 选答案，每题独立保存选择、自定义输入、字符光标和确认状态。
- Custom 是固定答案选项，下方输入行始终可用。预设答案可追加输入并形成 `预设\n输入`；Custom 要求非空且只提交输入。
- Enter 逐题确认并跳到下一未确认题；确认后仍可切回，任何改选或文本修改会撤销该题确认。全部题确认后才批量 resolve，模型 follow-up 同时获得完整答案集；Esc/Ctrl+D 作为当前题的批量跳过答案。
- 输入光标改用弹窗内容起点、`✎ ` 的实际两列宽度和 Unicode 显示列计算；长输入使用单行窗口保证光标始终位于边框内。
- 取消、异常 ToolEnd 和切换 session 会清理对应的全部 hub waiter，不残留 early answer。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 左右切题并保存逐题状态 | `arrows_switch_questions_and_preserve_each_questions_state` | `crates/tui/src/question_menu/state.rs` |
| 预设追加与 Custom 语义 | `preset_answer_appends_custom_input_but_custom_option_uses_only_input` / `custom_option_requires_text_and_enter_focuses_the_input` | `crates/tui/src/question_menu/state.rs` |
| 逐题确认、修改失效、批量跳过 | `confirmations_are_held_until_every_question_is_confirmed` / `editing_a_confirmed_question_requires_reconfirmation` / `skip_is_batched_with_answers` | `crates/tui/src/question_menu/state.rs` |
| 批量 resolve 与取消清理 | `no_question_resolves_until_the_complete_batch_is_confirmed` / `abandon_dialog_clears_all_waiters_without_early_answers` | `crates/tui/src/question_menu/mod.rs` |
| 空输入与 Unicode 精确光标 | `cursor_starts_after_the_input_prefix` / `cursor_uses_unicode_display_width_at_an_interior_position` / `long_input_window_keeps_cursor_inside_the_popup` | `crates/tui/src/question_menu/view.rs` |
| 多行粘贴保持单行光标模型 | `paste_keeps_the_custom_input_single_line_and_cursor_aligned` | `crates/tui/src/question_menu/state.rs` |
| worker 等齐全部答案后再进入 follow-up | `worker_waits_for_the_complete_question_batch_before_followup_context` | `crates/tui/tests/question_flow.rs` |

- 隔离 TUI 回归：`cargo test -p opencoder-tui` → 1266 unit tests + 62 integration tests + 0 doc tests，全部通过。
- 本任务 TUI clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告；`cargo build -p opencoder-tui` → 通过。
- 混合工作区 `cargo build --workspace` → 通过。
- 混合工作区 `cargo test --workspace` 被并发 store schema v9 改动阻断：`schema_v3_to_v4_adds_images_json_column` 实际版本为 9，但测试仍断言 8；本任务未修改该迁移逻辑。
- 混合工作区 clippy 被并发 `crates/todos/src/parent.rs::accept` 的 8 参数 `clippy::too_many_arguments` 阻断；本任务 TUI clippy 已独立通过。

## 兼容性
- question tool schema、session 并行执行和 headless/web 无监听兜底不变。
- `QuestionHub` 仍是 mid-turn 直连通道，不经 `UiCmd`。
