Commit: (working-tree, pre-initial-commit)

# feat(tui): 可折叠工具块 + 逐帧渲染残留清除 + 测试稳定性修复

## 背景

Tool 调用的输出（bash、read 等）在 TUI 中始终全量展开，长输出挤占视口。
Thinking 块已支持折叠，但 Tool 块缺少对称的折叠/点击展开能力。

同时，ratatui 双缓冲在第三帧复用第一帧 buffer 时，上一帧的残影会通过 diff
被重新画回屏幕（"render remnants around the edges"）。根因是缺少逐帧全区域
`Clear`。

此外 `open_store_creates_db_file_in_workdir_hashed_data_dir` 在全量并行测试下
偶发失败（`data_dir_for` 读取 HOME/XDG_DATA_HOME 环境变量，并发 env 变更导致
路径不一致）。`chat_tests.rs` 达 1393 行，超出 800 行迭代上限。

## 变更

### 可折叠工具块（`crates/tui/src/chat.rs` / `chat_types.rs` / `render.rs` / `render_hits.rs`）

- **`chat.rs::toggle_tool_at`**（行 301）：翻转指定 `Tool` 块的 `collapsed` 状态；
  越界或非 Tool 块时 no-op。
- **`chat.rs::collapse_all_collapsible`**（行 310）：一键折叠所有 `Thinking` + `Tool`
  块，绑定 Ctrl+L（`app_helpers.rs:139,147`）。
- **`chat_types.rs::ToolHeader`**：新增 `header_line_idx` 字段，记录 header 在
  `flatten()` 中的行号，用于鼠标命中测试。
- **`render_hits.rs::record_tool_hits`**（行 78）：镜像 `record_thinking_hits`，将
  每个 `ToolHeader` 映射到屏幕行，生成一行高的 `ToolBtn` 点击矩形。
- **`render.rs`**：`MouseHits` 新增 `tool_btns` 字段；点击事件遍历该列表调用
  `toggle_tool_at`（`app_helpers.rs:630-633`）。
- **Tool 块默认折叠**：`flatten()` 默认隐藏 Tool 输出（`collapsed = true`），仅显示
  header 行；输出保留完整文本（不再截断为 6 行），点击展开可见。

### 逐帧渲染清除（`crates/tui/src/render.rs` / `render_clear_tests.rs`）

- **`render.rs`**：`terminal.draw` 闭包顶部插入全区域 `Clear`，确保每帧从空白 buffer
  开始，消除 ratatui 双缓冲的残影。

### 测试稳定性与拆分

- **`app_helpers_tests/mod.rs`**：`open_store_creates_db_file_in_workdir_hashed_data_dir`
  新增 `HOME_TEST_LOCK` 串行化（行 272-275），消除并发 env 变更导致的路径漂移；
  `#[allow(clippy::await_holding_lock)]` 标注单线程测试运行时下持有锁的合法性。
- **`chat_tests.rs` → `chat_tests/` 目录模块**：1393 行拆为 7 个文件（均 ≤400 行）：
  `mod.rs`（288）、`tool_collapse.rs`（330）、`plan_card.rs`（310）、`thinking_state.rs`（153）、
  `subagent.rs`（132）、`agent_switch.rs`（105）、`image_render.rs`（94）。
  子模块统一使用 `use super::super::*;`（等价于原 `use super::*;` 的作用域）。
- **遗留测试重命名**：`collapse_all_thinking_collapses_every_block` →
  `collapse_all_collapsible_collapses_every_thinking_block`；
  `collapse_all_thinking_noop_without_thinking_blocks` →
  `collapse_all_collapsible_noop_without_collapsible_blocks`（匹配 `collapse_all_collapsible` 行为）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 逐帧 Clear 消除双缓冲残影 | `per_frame_clear_wipes_stale_glyphs_across_frames` | `render_clear_tests.rs` |
| Tool 输出保留完整、默认折叠 | `tool_output_retained_in_full_and_collapsed_by_default` | `chat_tests/tool_collapse.rs` |
| toggle_tool_at 展开后折叠 | `toggle_tool_at_expands_then_collapses` | `chat_tests/tool_collapse.rs` |
| toggle_tool_at 对非 Tool 块 no-op | `toggle_tool_at_is_noop_for_non_tool_blocks` | `chat_tests/tool_collapse.rs` |
| collapse_all_collapsible 折叠 Tool+Thinking | `collapse_all_collapsible_collapses_tools_and_thinking` | `chat_tests/tool_collapse.rs` |
| tool_headers 行号落在 header 行 | `tool_headers_line_index_lands_on_tool_header` | `chat_tests/tool_collapse.rs` |
| 折叠 header 可见时获得命中矩形 | `collapsed_header_visible_gets_hit_rect` | `render_tests.rs` |
| collapse_all_collapsible 折叠所有 Thinking | `collapse_all_collapsible_collapses_every_thinking_block` | `chat_tests/thinking_state.rs` |
| collapse_all_collapsible 无可折叠块时 no-op | `collapse_all_collapsible_noop_without_collapsible_blocks` | `chat_tests/thinking_state.rs` |

- 全量回归：`cargo test --workspace` → **1213 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished clean
- 行数：`chat_tests/*.rs` 均 ≤ 330 ≤ 400

## Impact Surface

- TUI 用户可点击 Tool 块 header 展开输出、Ctrl+L 一键折叠所有可折叠块。
- Tool 长输出不再截断，折叠态仅显示 header。
- 渲染残影消除。
- 不影响：Store/ChatStream/session/LLM 边界；CLI/Web 契约；runner/drain 语义。

## Related Docs

- [既有 changelog：queued-echo-marker](queued-echo-marker.md)
