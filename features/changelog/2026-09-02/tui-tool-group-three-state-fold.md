# TUI function call 组级三态折叠

Commit: (working-tree, TUI 工具调用组级三态折叠)

> 历史实现，当前交互已由 [Turn / Step / Function call 三级层级纠正](../2026-09-03/turn-step-function-call-hierarchy.md) 取代；下文保留当时设计与回归证据。

## 背景

- 原渲染模型是每个 tool call 一个 `ChatBlock::Tool` 块、一行 `▸ name args [↓ N]`，并行/连续调用在 transcript 里逐行铺开，视觉噪音大。
- 目标交互：**组级三态循环**——
  | 状态 | 渲染 | 进入方式 |
  |---|---|---|
  | Collapsed（默认） | 单行 `▸ N function calls`（只显示个数） | 默认 / Ctrl+L |
  | List | 组行 + 每个调用的 header（工具名+参数摘要，无输出） | 点击组行 |
  | Results | List + 每个调用的输出全文 | 再点击组行 |
  | （循环） | 回到 Collapsed | 第三次点击 / Ctrl+L |
- 分组单位 = **连续的 tool call 块**：一个 turn 内被 assistant 文本、Image、Marker 等非 Tool 块打断则另起一组。

## 变更

- `crates/tui/src/chat_types.rs`：`ChatBlock::Tool`（per-call `collapsed`）删除，替换为 `ChatBlock::ToolGroup { calls: Vec<ToolCall>, state: ToolGroupState }`；新增 `ToolGroupState`（Collapsed/List/Results，`Default = Collapsed`）与 `ToolCall { id, header, output, started_at_ms: Option, elapsed_ms: Option }`。**单一状态源**在组上，消灭双状态源错乱。
- `crates/tui/src/chat.rs`：
  - `apply`：`ToolStart` 尾块是 ToolGroup 则 append call（state 不动），否则新建 Collapsed 组；`ToolEnd` 按 id 倒序遍历组回填 output/elapsed（并发调用各归各位），无匹配走兜底「已完成单 call 组」（不再永久显示 running）；images 仍随后 push Image 块（自然截断组）。
  - `flatten_with` 三态渲染：Collapsed 单行（组内有未完成 call 时追加 `⠋ running` spinner 提示，用 `anim_tick`）；List = 组行 + 各 call header（2 空格缩进）+ 空行；Results 再加各 output + 每 call 尾空行。组行箭头 `▸`/`▾` 镜像展开态，List/Results 分别带 `[↓]`/`[↑]` 后缀提示。
  - `toggle_tool_at` → `cycle_tool_group_at`（三态循环）；`collapse_all_collapsible` 的 Tool arm 置 `Collapsed`（Ctrl+L 语义自动达成，键盘层零改动）。
  - Subagent 头部 tool 统计改为 `calls.len()` 求和。
- `crates/tui/src/chat_headers.rs`：行数核算同步三态——`1` / `1 + calls.len() + 1` / `1 + Σ(2 + output.len())`，与 flatten 严格一致（命中框对齐的关键约束，由 line_accounting 测试守卫）。
- `crates/tui/src/render.rs` + `render_hits.rs`：`ToolBtn` 指向组头行；第一版只注册组行点击（call 行不单独可点）。
- `crates/tui/src/app_helpers.rs`：`tool_btns` 命中改调 `cycle_tool_group_at`（subagent-aware `collapse_view` 机制复用）。
- `crates/tui/src/chat_helpers.rs`：`push_bash_tool`（`!cmd`）push 单 call 组且 **state=Results**（本地命令运行中输出可见的现状保持）；`finish_bash_tool` 回填后把组收为 Collapsed（等价旧「完成后折叠」）。
- `crates/tui/src/session_ui/replay.rs`：重放路径同样按「连续 ToolUse 归并成组、按 id 回填」；兜底块同步为单 call 组，`elapsed_ms: Some(0)` 保证 resume 后无 running 提示/垃圾计时。

## 已知边界

- **copy-mode**：copy-mode 直接渲染 flatten 行，组折叠时工具输出不可见（需先点开到 Results）——可接受，行为变化随本条目声明。
- **运行中的工具**：默认折叠会把「正在执行」藏进组行，用组行 `⠋ running` 提示补偿。
- **嵌套视图**：subagent/sidecar 的子 `ChatView` 复用同一渲染与折叠逻辑，自动获得新行为。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 并发 ToolStart 归并成一组、ToolEnd 按 id 各归各位 | `parallel_tool_calls_form_one_group_and_route_by_id` | crates/tui/src/chat_tests/tool_collapse.rs |
| 默认 Collapsed 单行只显示个数（含单数语法） | `collapsed_by_default_renders_single_count_line` | crates/tui/src/chat_tests/tool_collapse.rs |
| 组行 running 提示随完成消失 | `running_hint_in_group_line_while_call_unfinished` | crates/tui/src/chat_tests/tool_collapse.rs |
| 三态 cycle 各级行数（1 / 4 / 7 / 回 1） | `cycle_tool_group_line_counts_three_states` | crates/tui/src/chat_tests/tool_collapse.rs |
| 文本打断分组 + 旧组按 id 回填 | `text_between_calls_splits_groups_and_backfills_older_group` | crates/tui/src/chat_tests/tool_collapse.rs |
| 孤儿 ToolEnd 兜底单 call 组（已完成） | `orphan_tool_end_creates_synthetic_group` | crates/tui/src/chat_tests/tool_collapse.rs |
| 错误输出 err 着色 | `tool_end_error_colors_output_red` | crates/tui/src/chat_tests/tool_collapse.rs |
| 非 ToolGroup 块 cycle no-op | `cycle_tool_group_at_is_noop_for_non_tool_blocks` | crates/tui/src/chat_tests/tool_collapse.rs |
| Ctrl+L（collapse_all）收齐 Thinking+组 | `collapse_all_collapsible_resets_groups_and_thinking` | crates/tui/src/chat_tests/tool_collapse.rs |
| tool_headers 命中组行 | `tool_headers_line_index_lands_on_group_line` | crates/tui/src/chat_tests/tool_collapse.rs |
| bash 长命令不截断 | `summarize_keeps_full_bash_command_no_truncation` | crates/tui/src/chat_tests/tool_collapse.rs |
| 单次输出 200 行封顶 | `tool_output_truncated_at_limit` | crates/tui/src/chat_tests/tool_collapse.rs |
| `!cmd` 起始即 Results | `push_bash_tool_starts_in_results_state` | crates/tui/src/chat_tests/bash_tool.rs |
| `!cmd` 完成回填+收折+计时 | `finish_bash_tool_fills_output_and_collapses` | crates/tui/src/chat_tests/bash_tool.rs |
| `!cmd` 中止文案可见 | `finish_bash_tool_aborted_message` | crates/tui/src/chat_tests/bash_tool.rs |
| 三态行数核算 == flatten（命中框对齐） | `tool_group_three_state_alignment` | crates/tui/src/chat_tests/line_accounting.rs |
| running call 不额外占行 | `tool_group_running_call_keeps_alignment` | crates/tui/src/chat_tests/line_accounting.rs |
| 点击组行三态循环 + 未命中不误触 | `clicking_tool_group_line_cycles_three_states` | crates/tui/src/app_helpers_tests/mouse_tests.rs |
| Ctrl+L 父/子视图组全收起 | `ctrl_l_resets_tool_groups_to_collapsed` | crates/tui/src/app_helpers_tests/ctrl_l_tests.rs |
| 重放重建组（id/header/output 配对） | `replay_reconstructs_tool_blocks`、`replay_parallel_tools_paired_by_id`、`replay_tool_only_assistant_not_skipped` | crates/tui/src/session_ui.rs |
| 重放无 running 提示/垃圾计时（含孤儿兜底） | `replayed_tool_block_omits_duration_span`、`replayed_orphan_tool_result_omits_duration_span` | crates/tui/src/session_ui/replay_duration_tests.rs |
| 重放图片内联（组适配） | `replay_one_prefetched_tool_image_renders` 等 | crates/tui/src/session_ui/image_prefetch_tests.rs |
| 工具输出进入 copy/terminal 安全清洗路径 | `every_dynamic_chat_block_uses_the_same_terminal_safety_boundary` | crates/tui/src/chat_tests/terminal_safety.rs |
| 展开态不与 turn 计时混行 | `body_turn_cost_timer_not_mixed_into_tool_output` | crates/tui/src/render_tests/timer.rs |

## 回归

- `cargo test --workspace`：全绿（tui lib 1572 通过）。
- `cargo clippy -p opencoder-tui --all-targets`：0 警告；`cargo fmt --check`：干净。
