Commit: (working-tree)

# 重放路径重建 Tool 块 — replay_into_chat / replay_messages 不再丢弃工具调用

## 背景
会话恢复（`/task` 切回）和压缩（`TranscriptReset`）时，TUI 通过 `replay_into_chat` 从持久化消息重建 `ChatView`。子视图无持久化事件时通过 `replay_messages` 回退。两个函数**只提取 `ContentBlock::Text`**，丢弃全部 `ToolUse`/`ToolResult`——恢复后的会话不显示任何工具调用。

## 诊断
### 根因：filter_map 只留 Text + text.is_empty() 跳过 + Role::Tool 被 `_ => {}` 丢弃
- 两个函数用 `filter_map(|b| match b { ContentBlock::Text { text } => Some(...), _ => None })` 提取文本，`ToolUse` 块被丢弃。
- `if text.is_empty() { continue; }` 导致**纯工具调用**的 assistant 消息（无 Text 块）被整条跳过。
- `Role::Tool` 消息（携带 `ToolResult`，即工具输出）被 `_ => {}` 分支丢弃。
- 所需数据已在存储中：`ContentBlock::ToolUse.id` 与 `ContentBlock::ToolResult.tool_use_id`（core/src/message.rs:21-30），只是 TUI 重放代码未读取。
- 对比：`reconstruct_child_view` 主路径用 `view.apply(&ev)` 重放持久化事件——`apply` 有 `ToolStart`/`ToolEnd` 臂，**正确**处理工具；仅 `replay_messages` fallback 和 `replay_into_chat` 受影响。

## 变更
### `crates/tui/src/session_ui.rs`
- 新增 `replay_one(chat, msg)` 私有函数：按消息角色分发，镜像 `ChatView::apply` 的 `ToolStart`/`ToolEnd` 逻辑。
  - `Role::Assistant`：提取 Text 块 → `ChatBlock::Assistant`（done: true）；每个 `ToolUse` → `ChatBlock::Tool`（header 用 `summarize(input)`，空 output）。
  - `Role::Tool`：每个 `ToolResult` 按 `tool_use_id` 反向查找匹配的 `ChatBlock::Tool` 并追加输出（截断至 `TOOL_OUTPUT_LINES=6`）；无匹配则合成 `(output)` 孤儿块。
  - `Role::User`/`Role::System`：行为不变。
- `replay_into_chat` 的消息循环改为调用 `replay_one` + subagent 交错（`tasks_by_parent.remove(&msg.id)`）。纯工具 assistant 轮不再被跳过。
- `replay_messages` 的消息循环改为调用 `replay_one`。

### `crates/tui/src/chat.rs`
- `summarize` 改为 `pub(crate)`（供 `session_ui.rs` 构造 Tool 头部）。
- `TOOL_OUTPUT_LINES` 改为 `pub(crate)`（供 `session_ui.rs` 截断输出）。

### 涉及文件
- `crates/tui/src/session_ui.rs`（新增 `replay_one` + 重写 `replay_into_chat`/`replay_messages` 循环 + 3 测试）
- `crates/tui/src/chat.rs`（`summarize`/`TOOL_OUTPUT_LINES` 可见性）
- `crates/tui/src/chat_tests.rs`（+3 边界测试：孤儿 ToolEnd、error 着色、截断）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| **重放重建 Tool 块（ToolUse→ToolResult 配对，header+output 正确）** | `replay_reconstructs_tool_blocks` | `session_ui.rs` tests |
| **纯工具 assistant 轮不被跳过** | `replay_tool_only_assistant_not_skipped` | 同上 |
| **并行工具结果按 id 配对（乱序到达）** | `replay_parallel_tools_paired_by_id` | 同上 |
| **孤儿 ToolEnd（无 ToolStart）合成 (output) 块** | `orphan_tool_end_creates_synthetic_block` | `chat_tests.rs` |
| **is_error=true 输出着红色** | `tool_end_error_colors_output_red` | 同上 |
| **输出截断至 6 行** | `tool_output_truncated_to_six_lines` | 同上 |

## Impact Surface
- **行为改善**：会话恢复/压缩后正确显示工具调用；此前完全丢失。
- **渲染一致**：重放路径构造的 `ChatBlock::Tool` 与实时 `apply` 路径完全一致（相同 header 样式、相同截断、相同 id 路由）。
- **不影响**：`reconstruct_child_view` 主路径（已用 `apply` 正确处理）；快照恢复路径（克隆已有块）。
