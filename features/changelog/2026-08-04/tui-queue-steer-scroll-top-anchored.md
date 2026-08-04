# fix(tui): queue/steer 面板滚动改为 top-anchored（0 = 最旧条目）

## 背景

queue/steer 侧边栏的滚动偏移语义为 `0 = pinned to newest`（最新条目固定在顶部），
与常规滚动直觉相反（PageDown / 滚轮向下应看到更新的内容）。Shift+PageUp/PageDown
与鼠标滚轮方向也与此不一致，导致用户操作时方向混乱。

## 变更

统一为 **top-anchored**：`queue_scroll = 0` 时最旧条目在顶部，向下滚动看到更新的条目。

### 1. 渲染翻转 — `crates/tui/src/queue_panel.rs`

溢出窗口的计算从「偏移 = 跳过最新的 N 条」改为「偏移 = 跳过最旧的 N 条」，
滚动条 thumb 位置随之翻转（scroll=0 → thumb 在顶部）。

### 2. 键盘方向 — `crates/tui/src/key_handler.rs`

- `Shift+PageUp` → `saturating_sub`（向更旧的方向）
- `Shift+PageDown` → `saturating_add`（向更新的方向）

### 3. 鼠标滚轮方向 — `crates/tui/src/app_helpers.rs`

- `ScrollUp`（滚轮向上）→ `saturating_sub`（看更旧的条目）
- `ScrollDown`（滚轮向下）→ `saturating_add`（看更新的条目）
- 移除旧的 `max_scroll` clamp（基于 cached panel total），改为自然 floor at 0。

### 4. 注释对齐 — `crates/tui/src/app.rs`, `session_ui.rs`

注释从 `0 = pinned to newest` 更新为 `0 = pinned to top (oldest)`。

## 测试清单

| 路径 | 测试 | 文件 |
|------|------|------|
| TUI | `wheel_up_in_queue_panel_scrolls_panel_only` | `app_helpers_tests/mouse_wheel_tests.rs` |
| TUI | `wheel_down_advances_toward_newest` | `app_helpers_tests/mouse_wheel_tests.rs` |
| TUI | `wheel_outside_queue_panel_scrolls_body` | `app_helpers_tests/mouse_wheel_tests.rs` |
| TUI | `wheel_with_no_queue_panel_keeps_body_behavior` | `app_helpers_tests/mouse_wheel_tests.rs` |
| TUI | `shift_page_up_scrolls_queue_panel_not_body` | `key_handler_queue_scroll_tests.rs` |
| TUI | `shift_page_down_advances_toward_newest` | `key_handler_queue_scroll_tests.rs` |
| TUI | `shift_page_up_floors_at_zero` | `key_handler_queue_scroll_tests.rs` |
| TUI | `plain_page_up_still_scrolls_body` | `key_handler_queue_scroll_tests.rs` |
| TUI | `queue_panel_overflow_windows_and_scrollbar`（更新断言） | `render_tests/queue_panel.rs` |
| TUI | `queue_panel_overflow_hit_rects_track_shifted_glyphs`（更新断言） | `render_tests/queue_panel.rs` |

**当次实跑**: `cargo test --workspace` → 1839 passed; 0 failed。
`cargo clippy --workspace --all-targets` → 0 warning。
