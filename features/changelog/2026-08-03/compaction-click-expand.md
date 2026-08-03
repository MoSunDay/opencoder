# feat(tui): Compaction 块接入 click-to-expand 命中管线 + 折叠 header 清理

## 背景

Compaction（上下文压缩）块此前只读不可折叠——`CompactionDelta` 事件创建的
`ChatBlock::Compaction` 虽复用了 `render_collapsible`，但折叠 header 仍带冗余文字
`{icon} {label} (N lines) [↓ expand]` / `[{label}] [↑ collapse]`，且**无点击命中**：
鼠标点击无法展开/收起 Compaction 块（Thinking/Tool 块已有此能力）。本次让
Compaction 块与 Thinking 块行为对齐——同一 click-to-expand hit-rect 管线 + 清洁 header。

## 变更

### 折叠 header 清理（`crates/tui/src/compaction_block.rs`）
- `render_collapsible`（Thinking 与 Compaction 共用）header 文字精简：
  - 折叠态：仅 `{icon} {label}`（移除 `(count lines)` 行数与 `[↓ expand]` 提示）。
  - 展开态：仅 `{icon} {label}` italic-bold（移除 `[↑ collapse]` 提示）。
- 删除折叠态的 `count` 计算（已无需）。
- 文档注释补充：click-to-expand 由独立的 hit-rect 管线接线。

### 命中记录（`crates/tui/src/render_hits.rs`）
- 新增 `CompactionBtn { block_idx, rect }`（镜像 `ThinkingBtn`）。
- 新增 `record_compaction_hits(...)`：遍历 `chat.compaction_headers()`，将每个
  header 的逻辑行号经 `ViewportCache::row_of_line` 映射为屏幕行，视口内（含
  `scroll_y..scroll_y+visible_h` 裁剪）则推入一条 full-width (`text_w × 1`)
  `CompactionBtn`；滚出可见区（`>= viewport_bottom` 早退 / `< scroll_y` 跳过）不计。

### 渲染接入（`crates/tui/src/render.rs`）
- `MouseHits` 新增字段 `compaction_btns: Vec<CompactionBtn>`；每帧 `clear()`。
- `render_body` 签名新增 `compaction_btns: &mut Vec<CompactionBtn>` 出参，末尾调用
  `record_compaction_hits`。
- `pub(crate) use hit_records::CompactionBtn` 重导出，供 `app_helpers`/测试引用。

### 点击处理（`crates/tui/src/app_helpers.rs`）
- `handle_mouse` 新增 `compaction_btns` 点击循环（在 thinking/tool 循环同层）：
  `in_rect` 命中则对当前（sub）view 调 `ChatView::toggle_compaction_at(block_idx)`，
  置 `consumed = true`。

## 测试覆盖

新增 12 条测试（TUI lib 819 → **831**；`cargo test --workspace` 复跑 **1672** passed / 0 failed，已逐 binary 核对 failed 字段）：

| 层级 | 功能 | 测试名 | 文件 |
|------|------|--------|------|
| unit | 折叠 header 在视口内获得 full-width hit-rect | `collapsed_header_visible_gets_hit_rect` | `render_tests/compaction.rs` |
| unit | 展开后 header 屏幕行不变、内容可见 | `expanded_header_row_unchanged` | `render_tests/compaction.rs` |
| unit | header 滚出视口顶不产生命中 | `header_scrolled_above_is_not_hittable` | `render_tests/compaction.rs` |
| unit | 无 Compaction 块则无命中 | `no_compaction_blocks_means_no_hits` | `render_tests/compaction.rs` |
| unit | in_rect 命中 header 行、落空邻行 | `hit_rect_matches_click_on_header_row` | `render_tests/compaction.rs` |
| unit | 折叠 header 无行数/expand 文字 | `collapsed_header_has_no_extra_text` | `render_tests/compaction.rs` |
| unit | CompactionDelta 创建默认折叠块、内容隐藏 | `compaction_delta_creates_collapsed_block` | `chat_tests/compaction_state.rs` |
| unit | toggle 双向展开/收起 + last_compaction_collapsed | `toggle_expands_and_collapses` | `chat_tests/compaction_state.rs` |
| unit | collapse_all 覆盖 Compaction（与 Thinking/Tool 并列） | `collapse_all_covers_compaction` | `chat_tests/compaction_state.rs` |
| unit | 多 delta 累积进同一块 | `multiple_deltas_accumulate_in_one_block` | `chat_tests/compaction_state.rs` |
| unit | header 文字清洁（折叠+展开均无提示） | `header_text_is_clean` | `chat_tests/compaction_state.rs` |
| integration | 全链路点击 header → toggle 展开（内容可见性变化） | `compaction_header_click_toggles_collapse` | `app_helpers_tests/mouse_tests.rs` |

- 既有 `mouse_tests` 中两处 `MouseHits` 构造补 `compaction_btns: Vec::new()` 字段（结构体新增字段的连带修改，非新测试）。
- `render_collapsible` 被 Thinking + Compaction 共用：thinking 测试全绿确认 header 文字变更无回归。
- clippy（`--workspace --all-targets -D warnings`）：零警告。

## Impact Surface
- **行为变更**：Compaction 块现在可点击展开/收起；折叠/展开 header 文字更简洁。
  Thinking 块共享 `render_collapsible`，其 header 文字同步精简（同一函数）。
- **接缝不变**：未触及 `Store` / `ChatStream` / session / core / llm / store API 表面；
  变更完全隔离在 TUI 渲染 + 点击管线。
- `render_collapsible` 签名不变；`render_body` 新增尾部出参（内部 `pub(crate)`）。

## Related Docs
- [agents/tui](../../agents/tui/index.md) — MouseHits 命中管线、render_collapsible
