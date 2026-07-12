Commit: adb5ca4 (已提交)

# 并行工具输出按 id 路由到正确 ChatBlock — 修复 last_mut() 串扰

## 背景
当多个工具并行执行时（两个 `ToolStart` 后 `ToolEnd` 乱序到达），`ChatView::apply` 的 `ToolEnd` 分支使用 `self.blocks.last_mut()` 定位目标 Tool 块。这导致所有工具输出都被追加到最后一个压入的块，而非各自的块——输出串扰。

## 诊断
### 根因：ToolEnd 用 `last_mut()` 而非按 id 查找
- `ToolStart` 携带 `id`（tool-call id），但 `ChatBlock::Tool` 变体原本**没有 `id` 字段**，无法按 id 路由。
- `ToolEnd` 分支用 `self.blocks.last_mut()` 取最后一个块——并行场景下这是错误工具的块。
- 同文件的 `SubagentChild`（:211）和 `mark_subagent_done`（:585）均已用 `iter_mut().rev().find(|b| ... id == id)` 按 id 反向查找——三处路由点风格不一致。

## 变更
### `crates/tui/src/chat.rs`
- `ChatBlock::Tool` 变体新增 `id: String` 字段（与 `Subagent` 变体的 `id` 字段对齐）。
- `ToolStart` 分支：构造 `ChatBlock::Tool` 时设置 `id: id.clone()`。
- `ToolEnd` 分支：将 `self.blocks.last_mut()` 改为 `self.blocks.iter_mut().rev().find(|b| matches!(b, ChatBlock::Tool { id: bid, .. } if bid == id))`——按 id 反向查找，与 `SubagentChild`/`mark_subagent_done` 风格一致。
- 孤儿 `ToolEnd`（无匹配 `ToolStart`）的 fallback 分支也设置 `id: id.clone()`，合成 `(output)` 头块。
- `SteerConsumed { .. } => {}` 臂补全（`SessionEvent` 新增 variant 后 exhaustive match 需要）。
- 全部解构点用 `..` 丢弃 `id`，渲染路径不受影响。

### 涉及文件
- `crates/tui/src/chat.rs`（`ChatBlock::Tool` 定义 + `apply` 的 `ToolStart`/`ToolEnd`/`SteerConsumed` 臂）
- `crates/tui/src/chat_tests.rs`（回归测试 `parallel_tool_outputs_route_to_own_block`）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| **并行工具 end 乱序到达，各块只含自身输出** | `parallel_tool_outputs_route_to_own_block` | `chat_tests.rs` |

### 防伪绿验证
将路由改回 `last_mut()` 后，`parallel_tool_outputs_route_to_own_block` 立即在 `assert!(text_a.contains("A-out"))` 处失败——确认是真实回归测试。

## Impact Surface
- **回归守卫**：`ToolEnd` 的 id 路由契约被行为测试锁定。
- **渲染不变**：`flatten_with`/`thinking_headers`/`subagent_headers` 均用 `..` 丢弃 `id`，渲染路径不受影响。
- **全工作区仅 2 处构造 `ChatBlock::Tool`**，都已设 `id`。
