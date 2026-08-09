Commit: (working-tree, uncommitted)

# feat(tui): 统一用户消息为 `ChatBlock::User` markdown 块

## 背景

此前用户输入在 transcript 中缺乏与 `Assistant`/`Tool` 对等的可视边界：用户消息要么仅作历史记录、要么以非标准方式展示，标签样式与缩进不一致。本次将用户输入统一为 `ChatBlock::User{rendered}` 渲染块，与助手块对称，提升可读性与结构一致性。

## 变更

### `crates/tui/src/chat_types.rs`
- 新增 `ChatBlock::User { rendered: Vec<Line<'static>> }` 变体（markdown 预渲染正文）。
- 新增共享纯函数 `indented(rendered, width)`：为每行前置定宽空格缩进，供 `Assistant`/`User`/`Image` 三处 flatten 复用，杜绝缩进实现分叉。

### `crates/tui/src/chat.rs`
- `flatten_with` 新增 `User` arm：先压金色（`theme::user_color`）加粗 `❯ User:` 标签，再 `types::indented(rendered, 4)` 输出 4 空格缩进正文。
- `Assistant` arm 标签由小写改为加粗 `❯ Say:`（绿色 `theme::ok_color`），与 `❯ User:` 对称；缩进正文改用共享 `types::indented`。
- `SteerConsumed` 分支：消耗的 steer 提示经 markdown 渲染后推入 `ChatBlock::User`（先前为 marker/裸文本），使 steer 文本与正常提交共享同一展示模型。
- `collect_headers` 与 line-accounting mirror 同步新增 `User` 分支（1 标签 + rendered.len() 行），保证可见行数与计数口径一致。

### `crates/tui/src/chat_headers.rs` / `app_helpers.rs` / `app_loop.rs` / `session_ui/replay.rs`
- `chat_headers.rs`：`collect_headers` 增加 `User` 分支。
- `app_helpers.rs::push_user`：构造 `ChatBlock::User`（markdown 渲染正文）。
- `app_loop.rs`：`QueueConsumed` 路径使用 `push_user` 统一入块。
- `session_ui/replay.rs`：`Role::User` 回放经 `push_user` 渲染，与 live 路径一致。

### `crates/tui/src/theme.rs`
- `user_color()` 返回 `palette(current_theme()).user`（暗 Indexed(220) / 亮 Indexed(94)），是 `User` 块标签色源。
- **修复 flaky 测试**：`user_color_is_gold_in_dark_theme` / `user_color_is_dark_gold_in_light_theme` 不再经全局 `THEME`（`set_theme` + `user_color()` 两把锁之间存在 race，并行下 ~1/5 失败），改为直接断言纯函数 `palette(ThemeKind::*).user`，零全局状态、确定性。

### 颜色与样式
- `❯ User:`：`theme::user_color()` + BOLD（暗 Indexed(220) / 亮 Indexed(94)）。
- `❯ Say:`：`theme::ok_color()` + BOLD（绿色）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| User 块渲染金色 `❯ User:` 标签 + 4 空格缩进正文（断言标签 span fg = user_color、正文 4 空格前缀） | `user_block_renders_gold_tag_and_indented_body` | `crates/tui/src/chat_tests/user_block.rs` |
| User 块行数与 `collect_headers` 计数一致（1 标签 + rendered.len()） | `user_block_line_count_matches_collect_headers` | `crates/tui/src/chat_tests/user_block.rs` |
| `push_user` 生成含 markdown 正文体的 `ChatBlock::User` | `push_user_creates_user_block_with_markdown` | `crates/tui/src/chat_tests/user_block.rs` |
| SteerConsumed 消耗时回显 `ChatBlock::User` 块并清行 | `steer_consumed_echoes_marker_and_drops_row` | `crates/tui/src/chat_tests/steer_echo.rs` |
| 混合序列（含 User）行数对齐 | `mixed_sequence_alignment` | `crates/tui/src/chat_tests/line_accounting.rs` |
| plan 卡片中 SteerConsumed 回显 User 块 | `steer_consumed_echoes_marker_and_drops_entry` | `crates/tui/src/chat_tests/plan_card.rs` |
| `push_user` 记录历史并回显 transcript | `push_user_records_history_and_echoes_transcript` | `crates/tui/src/app_helpers_tests/mod.rs` |
| user_color 暗主题为金色（纯函数，确定性） | `user_color_is_gold_in_dark_theme` | `crates/tui/src/theme.rs` |
| user_color 亮主题为暗金色（纯函数，确定性） | `user_color_is_dark_gold_in_light_theme` | `crates/tui/src/theme.rs` |

## 备注

- `chat.rs` 的 `recover_round_anchor_if_missing()` 及 `chat_helpers.rs` 的 `reconcile_orphaned_subagents()`（UI 通道饱和丢 lifecycle 事件的自愈）见同日 changelog `tui-chat-self-heal-dropped-events.md`，本次随 TUI 一并提交。
