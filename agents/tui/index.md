Commit: (working-tree, pre-initial-commit)

# tui 模块

## 职责
ratatui + crossterm 交互界面。3-region 布局、事件循环、鼠标命中测试、键盘分发、subagent 可折叠 + ctx 切换。

## 边界与非目标
- 不直接调 Store CRUD（经 worker `UiCmd` 通道）；不持有 SessionState（worker 持有）。
- 非目标：TUI 不是 Web 替代品——无 SSE replay，仅 live event 流。

## 关键抽象
- `ChatView`（`src/chat.rs`）：`blocks: Vec<ChatBlock>` + `agent/status/subagents_running/subagents_total`。`apply(&SessionEvent)` 逐事件更新——核心接缝。
- `ChatBlock`（`src/chat.rs`）：`Marker(Vec<Line>)` / `Assistant{raw,rendered,done}` / `Thinking{text,collapsed}` / `Tool{header,output}` / `Subagent{id,child_session_id,kind,prompt,collapsed,view:ChatView,done,ok,summary}`。`flatten()` → `Vec<Line>` 供 Paragraph 渲染。
- `Subagent` block 可折叠（默认折叠）：折叠时 `▸ ⇲ subagent [explore] prompt [running, N tools]`；展开时显示子 ChatView 内容 + 完成摘要。点击表头折叠→展开，再点进入子会话 ctx 视图（body 渲染子 ChatView），Esc 返回。
- `MouseHits`（`src/render.rs`）：每帧重算的命中目标——`jump_btn/body/queue_btns/thinking_btns/subagent_btns`。`record_thinking_hits` / `record_subagent_hits` 映射逻辑行号到屏幕行号。
- `key_handler`（`src/key_handler.rs`）：`KeyAction` enum + `handle_key()` 分发器 + `move_hist()` 历史导航。从 app.rs 提取（行数控制）。
- 手动 `draw_scrollbar()`（`src/render.rs`）：替代 ratatui ScrollbarState——简单比例 `scroll_y * max_off / max_scroll`，短内容对齐正确。滚轮处理器换行宽度 `r.width - 3` 与 render 的 `text_w = inner.width - 1` 对齐。

## 主流程
`run_app` loop：每帧 `render()` → `tokio::select!` 三臂（crossterm 事件 / worker `evt_rx` / anim ticker）。

- **render**：`display_chat` = subagent_focus 时取子 ChatView，否则取 `&chat`。`display_agent` = focus 时 `← [Esc] back | ⇲sub [kind] prompt`。
- **Session 事件**：`track_context` 递归处理 SubagentChild（子事件 token 计入 ctx 仪表）；`chat.apply(&sev)` 路由 SubagentChild 到子 block view；TranscriptReset 重建 ChatView。
- **subagent ctx 切换**：`subagent_focus: Option<usize>` 状态。进入时保存 `parent_scroll/parent_follow`，退出时恢复。Esc 优先拦截（在 handle_key 之前）。
- **输入模式**：Enter = steer（运行中）/ submit（idle）；Tab = followup（运行中）/ submit（idle）；Shift+Tab 切 plan/act。

## 依赖与接口
- 依赖：ratatui 0.29、crossterm、tokio、opencode-session、opencode-core、opencode-store、opencode-llm（estimate）。
- 被依赖：binary crate（`src/main.rs` → `opencode_tui::run_tui`）。
- worker 通道：`UiCmd::{Prompt,SwitchAgent,SwitchAndStart,Compact,SetSkill,ReloadConfig,Quit}` → `UiEvent::{Session(SessionEvent),TurnDone}`。

## 相关模块
- [agents/session](../session/index.md) — SessionEvent 来源。
- [agents/core](../core/index.md) — ChatView 依赖 Message/ContentBlock。

## 代表性锚点
- ChatBlock/Subagent 可折叠测试：`chat::tests::subagent_events_render`
- thinking 命中测试一致性：`chat::tests::thinking_headers_match_flatten_line_indices`
- worker cancel-token 交换回归：`worker::tests::rebind_session_swaps_the_active_cancel_token`
