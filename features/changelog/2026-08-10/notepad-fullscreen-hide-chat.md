Commit: c14cddb92700369ad3a070d3d7bca7820d7d98e

# Notepad 全屏编辑（隐藏底部聊天区 + 移除 Ctrl+O 焦点切换）

## Context

`/notepad` 打开时是「上 notepad + 下 chat」的分屏布局，底部常驻聊天 body/composer/status，编辑文件时占用视线与注意力。需求：notepad 打开即全屏接管终端，纯文件查看/编辑；同时**移除**分屏唤出能力（`toggle_focus` Ctrl+O 快捷键一并删除），`Esc` 直接退回正常聊天视图。

## Change Summary

- **全屏渲染**：`render.rs::render` 的 notepad 分支不再 split，`render_top` 画满整个 `f.area()` 后 `return`，跳过 body/composer/status/popups/光标全部聊天渲染；进入 notepad 分支时清空全部 chat hit-target（body/jump/top/queue_panel/total_rows/queue_btns/thinking/subagent/tool/compaction），避免上一帧残留命中触发误滚动/误点。
- **移除 split 机制**：`notepad/mod.rs` 删除 `layout_split`/`render_divider`/`MIN_BOTTOM`/`height` 字段与 `NotepadOutcome::FocusChat` 变体；`app_notepad.rs` 删除 `handle_notepad_drag`（divider 拖拽）及其测试。
- **移除 Ctrl+O 焦点切换**：`app_notepad::key` 不再读 `toggle_focus`，签名去掉 `keymap`/`np_chat_focus`；`app.rs`/`app_loop.rs` 删 `np_chat_focus`/`np_drag` 状态；keymap 配置 `toggle_focus` 字段从 `KeymapConfig`/`KEYMAP_INFO`/`KeyBindings` 全量移除（21→20 项），帮助文本删除 Ctrl+O 行。
- **翻页精确性**：`notepad/keys.rs::editor_inner_height` 全屏下返回 `终端高度-2`（与 `render_editor` 的 `block.inner` 一致），Ctrl-D/U/F/B 翻页与 `ensure_cursor_visible` 定位精确。
- **未改动**：`Store`/`ChatStream` 抽象、session 逻辑、tree/editor/search 内部行为、集成测试 `NotepadView::new` 调用方（`new()` 无 height 字段，签名不变）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 全屏不渲染 chat + hits 清空 | `render::clear_tests::notepad_fullscreen_hides_chat_and_clears_hits` | `crates/tui/src/render_clear_tests.rs` |
| notepad key 分发（无 toggle） | `app_notepad::tests::key_consumed_when_notepad_open` | `crates/tui/src/app_notepad.rs` |
| Ctrl+O 无特殊语义（notepad 保持打开） | `app_notepad::tests::ctrl_o_has_no_special_meaning` | `crates/tui/src/app_notepad.rs` |
| Esc 退出 notepad | `app_notepad::tests::esc_closes_notepad` | `crates/tui/src/app_notepad.rs` |
| 未打开 notepad 时 key 不拦截 | `app_notepad::tests::key_unhandled_when_notepad_closed` | `crates/tui/src/app_notepad.rs` |
| keymap 配置项计数（21→20） | `keymap_info_count_matches_fields` | `crates/core/src/config/keymap.rs` |
| keymap 菜单条目计数 | `keymap_menu::state::tests::new_menu_has_20_entries` / `navigate_up_wraps` / `navigate_down_wraps` | `crates/tui/src/keymap_menu/state.rs` |
| 全屏翻页精度（th-2）+ 光标定位 | `notepad::keys::tests::editor_page_down_uses_fullscreen_height` | `crates/tui/src/notepad/keys.rs` |

- 全量回归：`cargo test --workspace` → 2291 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
- [Features 索引](../../../features/index.md)
