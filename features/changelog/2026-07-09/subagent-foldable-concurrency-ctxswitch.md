Commit: (working-tree, pre-initial-commit)

# subagent 并发修复 + 可折叠 + ctx 切换 + 计数

## 背景
用户报告三个问题：
1. subagent 没有并发执行
2. subagent 应该像思考一样可折叠，点进去后看执行状态，包括 ctx 都要切过去，要能看到 subagent 个数
3. 需要测试用例

## 变更

### Part A — 并发保证（runner.rs + mock.rs + agent.rs）
- **yield_now 保证**（`runner.rs`）：在 `FuturesUnordered` 每个 future 开头加 `tokio::task::yield_now().await`，确保第一次 poll 后所有 future 都被调度到，不会因快速响应导致串行执行。
- **mock 让出**（`mock.rs`）：mock 每发一个 event 前先 `yield_now().await`，使 `recv()` 返回 `Pending`，让并发测试能验证真实交错。
- **prompt 强化**（`agent.rs`）：BASE_PROMPT 增加「You MAY emit multiple `task` blocks in a single response. Independent subagents dispatched this way run concurrently, so prefer batching independent investigations.」
- **测试加强**（`subagent.rs`）：`concurrent_subagent_dispatch_in_one_turn` 新增断言——第二个 SubagentStart 必须在第一个 SubagentEnd 之前到达（证明并发交错）。

### Part B — 事件层：child_session_id + SubagentChild wrapper（runner.rs）
- **`SubagentStart` 新增 `child_session_id: String`**：携带子会话 ID，供 TUI 加载子记录用。
- **`SessionEvent::SubagentChild { id: String, ev: Box<SessionEvent> }`**：新事件包装器。子回调不再仅转发 ToolStart/ToolEnd 到父 transcript，而是将**所有**子事件（TextDelta、ReasoningDelta、ToolStart、ToolEnd、Done、Error）包装在 `SubagentChild` 中转发，供 TUI 路由到子 block 的独立 ChatView。
- 子事件持久化不受影响——子事件仍按 child_session_id 写入 session_events。
- 更新 web/handle.rs、cli/run.rs、测试 format_ev 以处理新变体。

### Part C — ChatBlock::Subagent 可折叠变体（chat.rs）
- **`ChatBlock::Subagent { id, child_session_id, kind, prompt, collapsed, view: ChatView, done, ok, summary }`**：可折叠 block，内含子会话的独立 ChatView（从 SubagentChild 事件实时构建）。
- 折叠时显示 `▸ ⇲ subagent [explore] prompt... [running, 3 tools]`；展开时显示子 ChatView 内容 + 完成摘要。
- `SubagentStart` → 创建 block（默认折叠）；`SubagentChild` → 路由到 block 的 view；`SubagentEnd` → 标记完成。
- `subagents_total: u32` 新增到 ChatView（总数 = 运行中 + 已完成）。
- `toggle_subagent_at()` / `subagent_headers()` 镜像 thinking block 模式。

### Part D — ctx 切换 + 鼠标处理 + 计数（render.rs + app.rs）
- **ctx 切换**：点击 subagent header 第一次展开（折叠→展开），第二次（已展开）进入子会话视图——body 渲染子 ChatView，标题显示 `← [Esc] back | ⇲sub [explore] prompt`。Esc 退出。
- **鼠标命中测试**：`MouseHits` 新增 `subagent_btns` + `SubagentBtn` struct + `record_subagent_hits()` 函数（镜像 `record_thinking_hits`）。
- **计数徽章**：状态栏从 `⇲sub:{running} running` 改为 `⇲sub:{running}/{total}`，total > 0 时始终显示。
- **`track_context`** 递归处理 `SubagentChild`——子事件的 token 也计入 ctx 仪表。
- **模块拆分**：`handle_key` + `KeyAction` + `move_hist` 从 app.rs（832 行超限）提取到新文件 `key_handler.rs`（193 行），app.rs 降至 650 行。

### Part E — 模块拆分
- `crates/tui/src/key_handler.rs`（新文件，193 行）：`KeyAction` enum + `handle_key` 函数 + `move_hist` 辅助函数 + `ESC_CANCEL_WINDOW_MS` 常量。

## 测试
| 测试 | 文件 | 覆盖点 |
|------|------|--------|
| `estimate_messages_counts_tool_results_and_tool_use` | `tokens.rs` | 压缩估算含 ToolResult |
| `concurrent_subagent_dispatch_in_one_turn`（加强） | `subagent.rs` | 并发交错断言 |
| `subagent_wraps_child_events_in_subagent_child` | `subagent.rs` | SubagentChild 包装 |
| `subagent_events_render`（加强） | `chat.rs` | 可折叠 + 计数 + ctx 路由 |

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 260 passed | 261 passed |
