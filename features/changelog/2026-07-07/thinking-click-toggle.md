Commit: (working-tree, pre-initial-commit)

# thinking 块点击展开/收起（TUI 鼠标接入 + Web 折叠）

## Context

TUI 早有 `Thinking { text, collapsed }` 块且默认折叠，但 `toggle_last_thinking()` 是**死代码**——没有鼠标/键盘入口调用它，用户**永远无法展开**思考内容。`features/changelog/2026-07-05/tui-major-overhaul.md` 声称「可折叠（点击表头切换）」，实际从未接线。Web 端（`manager.html`）则把 reasoning 原样铺成 `… text` 单行，无折叠。

本变更补齐缺失的点击入口：TUI 点击表头行切换该块折叠/展开，Web 端同样默认折叠 + 点击展开。保留**默认折叠**作为渲染性能优化——折叠态下 thinking 内容不进 `flatten()` 输出，每帧渲染/换行开销为 O(1)/块；仅当用户点击展开某个块时，该块内容才进入渲染路径，且只影响被点的那一块。

## 变更

### TUI（`crates/tui`）
- **`chat.rs`**：
  - 新增 `ThinkingHeader { block_idx, header_line_idx }` + `ChatView::thinking_headers()`——按 `flatten()` 同样的逐块行计数遍历，返回每个 Thinking 块表头在扁平 `Vec<Line>` 中的行索引（与渲染保持同步）。
  - 新增 `ChatView::toggle_thinking_at(block_idx)`——按 block 索引精确切换折叠（替代死代码 `toggle_last_thinking()`，后者只切最后一个且无调用方）。越界或非 Thinking 索引为 no-op。
  - 保留 `collapsed` 字段与 `ensure_thinking_open()` 默认折叠不变。
- **`render.rs`**：
  - `MouseHits` 增 `thinking_btns: Vec<ThinkingBtn{block_idx, rect}>`，每帧 `clear()` 后由 `render_body` 填充。
  - 新增 `record_thinking_hits()`——按表头行索引增量累计 wrapped 屏幕行（`wrapped_rows()` 复用 `Paragraph::line_count` 保证与 ratatui 换行一致），仅对落在视口 `[scroll_y, scroll_y+visible_h)` 内的表头记录全宽 1 行 hit rect；越过视口底部即停。**无 Thinking 块时仅一次空 `thinking_headers()` 调用，零额外开销。**
- **`app.rs`**：`MouseEventKind::Down(Left)` 在 jump_btn / queue_btns 之后追加 thinking_btns 循环——命中即 `chat.toggle_thinking_at(block_idx)`。

### Web（`crates/web/src/manager.html`）
- reasoning 块由 `… text` 单行改为 `.think` 容器：表头 `💭 Thinking (N lines) [↓ expand]`（点击切换）+ 默认 `display:none` 的内容子节点；展开后表头变 `💭 Thinking [↑ collapse]`。新增 `.think/.th/.tb` 样式（暗色斜体、pointer 光标）。默认折叠，与 TUI 一致。

## 性能要点
- 默认折叠保持每帧 `flatten()`/换行/渲染对 thinking 内容 O(1)/块。
- 命中检测成本正比于「视口附近的 Thinking 表头数」（典型 0–2 个 `line_count` 调用），而非整个 transcript；无可折叠块时≈0。
- 与现有 `record_*_hits`（queue_btns）同模式：每帧重建、随 render 一次性算完。

## 涉及文件
- `crates/tui/src/chat.rs` — `ThinkingHeader` + `thinking_headers()` + `toggle_thinking_at()`；删 `toggle_last_thinking()`；+2 测试
- `crates/tui/src/render.rs` — `MouseHits.thinking_btns` + `ThinkingBtn` + `record_thinking_hits()` + `wrapped_rows()`；+5 测试（render::tests 模块新增）
- `crates/tui/src/app.rs` — 鼠标左键点击分发
- `crates/web/src/manager.html` — reasoning 默认折叠 + 点击展开

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| thinking_headers 行索引与 flatten 一致 | `thinking_headers_match_flatten_line_indices` | `tui/src/chat.rs` |
| 按 block 索引精确切换（不影响其它块/越界 no-op） | `toggle_thinking_at_toggles_specific_block` | `tui/src/chat.rs` |
| 折叠表头可见时生成全宽 hit rect | `collapsed_header_visible_gets_hit_rect` | `tui/src/render.rs` |
| 展开后表头屏幕行不变 | `expanded_header_row_unchanged` | `tui/src/render.rs` |
| 表头滚出视口上方不可点 | `header_scrolled_above_is_not_hittable` | `tui/src/render.rs` |
| 无 Thinking 块→零 hit | `no_thinking_blocks_means_no_hits` | `tui/src/render.rs` |
| hit rect 命中表头行、不命中邻行 | `hit_rect_matches_click_on_header_row` | `tui/src/render.rs` |

- 全量回归：`cargo test --workspace` → 全绿（tui 101 passed 含 7 新增；其余 crate 全过）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数 gate：`chat.rs` 545、`render.rs` 574、`app.rs` 788（均 ≤800）

## 相关文档
- 早期声称但未接线：[tui-major-overhaul](../2026-07-05/tui-major-overhaul.md)
- 同类点击 hit-rect 模式（queue 面板）：[newline-fallback-and-queue-reorder](./newline-fallback-and-queue-reorder.md)
