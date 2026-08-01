Commit: (working-tree, pre-initial-commit)

# feat(tui): 队列/转向面板可滚动（Shift+PageUp/Down 与滚轮）+ 右列滚动条

## 背景

排队/插队项唯一可见面是侧边队列面板（`queued:`/`steer:` 消费标记）。此前面板
只显示最新一屏，条目超过面板高度时更早的排队项完全不可见，也无法区分"新到的
项"与"面板窗口内最旧项"。本轮为面板增加独立滚动偏移 `queue_scroll`（0 = 钉住
最新），支持 Shift+PageUp/PageDown 与滚轮滚动，并在面板右列绘制滚动条。

## 变更

### 纯函数窗口数学（`crates/tui/src/queue_panel.rs`）

- `visible_window(total, height, scroll) -> (start, max_scroll, overflow)`：
  计算面板可见窗口起点与合法滚动上限；`scroll` 越界（stale，如面板隐藏期间
  残留）时钳制回合法区间——`total ≤ height` 时恒 `(0, 0, false)`（钉住最新）。
- `draw_queue_scrollbar`：面板右列（`x = width-1`）渲染 █ 拇指 + ┊ 轨道；
  溢出时控制钮列整体左移 1 列，命中矩形同步跟随（`btn_x_offsets(width-1)`）。
- `render_queue_panel` 新增 `scroll: u32` 参数，溢出分支按 `visible_window`
  结果切片渲染；`queue_scroll` 由 render 侧先钳制再传入。

### 输入路径（`crates/tui/src/key_handler.rs`、`app_helpers.rs`）

- **Shift+PageUp / Shift+PageDown**：`handle_key` 新增分支——`queue_scroll` 加/减
  1（`saturating`），返回 `KeyAction::None`；普通 PageUp/PageDown 仍滚动正文。
  键位提示同步加入 `keybind.rs` HELP。
- **滚轮**：`handle_mouse` 的 ScrollUp/ScrollDown 分支在命中
  `hits.queue_panel` 时只改 `queue_scroll`（上滚 +1 且钳制到
  `queue_total - panel_height`，下滚 `saturating_sub(1)` 回最新），正文
  `scroll`/`follow` 不被触碰；面板 rect 为 None（plan 模式隐藏）时保持正文
  行为，stale `queue_scroll` 由 render 钳制、状态无害存活。

### 状态接线（`crates/tui/src/app.rs`、`render.rs`、`session_ui.rs`）

- `run_app` 新增 `queue_scroll: u32` 状态（与 `scroll`/`follow` 并列），贯穿
  `render_frame` / `handle_key` / `handle_mouse` 三处调用点。
- `render.rs`：`queue_scroll` 传入前按面板 `max_scroll` 钳制 stale 偏移。
- `SessionUiState` 增加 `queue_scroll` 字段——`/task` 切换与 quit→resume 时
  面板滚动位置随会话快照保存/恢复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 窗口数学：不溢出钉住最新 / stale 钳制 | `visible_window_pins_to_newest_when_fits`、`visible_window_clamps_stale_scroll` | crates/tui/src/queue_panel.rs |
| Shift+PageUp 只滚面板不滚正文 | `shift_page_up_scrolls_queue_panel_not_body` | crates/tui/src/key_handler_queue_scroll_tests.rs |
| Shift+PageDown 回最新 / 在 0 处不动 | `shift_page_down_returns_to_newest`、`shift_page_down_floors_at_zero` | crates/tui/src/key_handler_queue_scroll_tests.rs |
| 无 Shift 的 PageUp 仍滚正文（回归守卫） | `plain_page_up_still_scrolls_body` | crates/tui/src/key_handler_queue_scroll_tests.rs |
| 滚轮：面板上滚只看更早、下滚回最新、不碰正文 | `wheel_up_in_queue_panel_scrolls_panel_only`、`wheel_down_in_queue_panel_returns_toward_newest`、`wheel_outside_queue_panel_scrolls_body`、`wheel_with_no_queue_panel_keeps_body_behavior` | crates/tui/src/app_helpers_tests/mouse_wheel_tests.rs |
| 渲染：溢出窗口 + 滚动条拇指顶/底位置 | `queue_panel_overflow_windows_and_scrollbar` | crates/tui/src/render_tests/queue_panel.rs |
| 渲染：控制钮/命中矩形随滚动条左移 1 列 | `queue_panel_overflow_hit_rects_track_shifted_glyphs` | crates/tui/src/render_tests/queue_panel.rs |
| 快照恢复：queue_scroll 随会话保存 | `session_ui.rs::tests::snapshot_*`（`snap.queue_scroll` 断言） | crates/tui/src/session_ui.rs |

- 全量回归：`cargo test --workspace` → **1587 passed / 0 failed / 1 ignored**（当次实跑；ignored 为既有 `research_smoke_bing_wikipedia`，需真实 Chrome/网络）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`queue_panel.rs` 441、`render.rs` 786、`app.rs` 798、`app_helpers.rs` 791、`session_ui.rs` 797 ≤ 800（迭代）；新文件 `mouse_wheel_tests.rs` 223、`mouse_scroll_tests.rs` 201、`key_handler_queue_scroll_tests.rs` 177 ≤ 400（拆分自 `mouse_tests.rs` 1057→461、`key_handler_tests.rs` 863→695，按 `#[cfg(test)] #[path]` 约定注册）

## Impact Surface

- **可感知影响**：队列面板可滚动查看更早的排队/插队项；Shift+PageUp/Down 与
  滚轮在面板上滚动面板、正文不受影响；滚动位置随 `/task` 切换恢复。
- **不影响**：正文滚动语义、store 形状、session runner / web / CLI headless；
  面板隐藏（plan 模式）时 stale 偏移无害存活并在下次渲染钳制。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](./queued-combined-skill-display.md)
