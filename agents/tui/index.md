Commit: (working-tree, pre-initial-commit)

# tui 模块

## 职责
ratatui + crossterm 交互界面。3-region 布局、事件循环、鼠标命中测试、键盘分发、subagent 可折叠 + ctx 切换。

## 边界与非目标
- 不直接调 Store CRUD（经 worker `UiCmd` 通道）；不持有 SessionState（worker 持有）。
- 非目标：TUI 不是 Web 替代品——无 SSE replay，仅 live event 流。

## 关键抽象
- `ChatView`（`src/chat.rs`）：`blocks: Vec<ChatBlock>` + `agent/status/subagents_running/subagents_total/context_used`。`apply(&SessionEvent)` 逐事件更新——核心接缝；内部 `track_context` 累加 transcript token，递归 SubagentChild（父 view 的 context_used 含全部后代 token，子 view 独立维护自身子树）。app.rs 不再有独立 `context_used` 变量——直接读写 `chat.context_used`。
- `ChatBlock`（`src/chat.rs`）：`Marker(Vec<Line>)` / `Assistant{raw,rendered,done}` / `Thinking{text,collapsed}` / `Tool{header,output}` / `Subagent{id,child_session_id,kind,prompt,view:ChatView,done,ok,summary}`。`flatten()` → `Vec<Line>` 供 Paragraph 渲染。
- `Subagent` block 不可内联展开：始终单行表头 `⇲ subagent [kind] prompt [●/✔/✘ status, N tools] [→ view]`（running=黄● / done=绿✔ / failed=红✘）。点击表头直接进入子会话 ctx 视图（body 渲染子 ChatView + 子 view 自有 context_used 驱动状态栏 ctx 仪表），Esc 返回主进程。
- `MouseHits`（`src/render.rs`）：每帧重算的命中目标——`jump_btn/body/queue_btns/thinking_btns/subagent_btns`。`record_thinking_hits` / `record_subagent_hits` 映射逻辑行号到屏幕行号。
- `key_handler`（`src/key_handler.rs`）：`KeyAction` enum + `handle_key()` 分发器 + `move_hist()` 历史导航。从 app.rs 提取（行数控制）。Ctrl+C/Ctrl+D 在 CONTROL 块同时匹配 `Char('c')|Char('d')` 和 Kitty 键盘协议下的原始控制字符 `Char('\u{3}')|Char('\u{4}')`——后者由 `DISAMBIGUATE_ESCAPE_CODES` 导致，不带 'c'/'d' 字面量。
- 手动 `draw_scrollbar()`（`src/render.rs`）：替代 ratatui ScrollbarState——简单比例 `scroll_y * max_off / max_scroll`，短内容对齐正确。滚轮处理器换行宽度 `r.width - 3` 与 render 的 `text_w = inner.width - 1` 对齐。

## 主流程
`run_app` loop：每帧 `render()` → `tokio::select!` 三臂（crossterm 事件 / worker `evt_rx` / anim ticker）。crossterm 事件臂 `events.next()` 返回 `None`/`Err`（流关闭，如终端 EOF）时直接 `UiCmd::Quit` + break，防止死流忙循环。

- **render**：`display_chat` / `display_agent` / `display_ctx` / `display_sys` = subagent_focus 时取子 ChatView 及其 `context_used` + 缓存的 `subagent_sys`（进入时 `sys_tokens_for(kind, workdir, None)` 算一次），否则取 `&chat` + `chat.context_used` / `sys_tokens`。`display_agent` = focus 时 `← [Esc] back | ⇲sub [kind] prompt`。
- **Session 事件**：`chat.apply(&sev)` 内部调 `track_context` 更新 `chat.context_used`（递归 SubagentChild 使父级含后代 token），SubagentChild 路由到子 block view（子 view 的 apply 独立更新自身 context_used）；TranscriptReset 重建 ChatView（context_used 归零）。
- **subagent ctx 切换**：`subagent_focus: Option<usize>` 状态。进入时保存 `parent_scroll/parent_follow`，退出时恢复。Esc 优先拦截（在 handle_key 之前）。
- **输入模式**：Enter = steer（运行中）/ submit（idle）；Tab = followup（运行中）/ submit（idle）；Shift+Tab 切 plan/act。
- **中止与续跑**：双击 Esc 硬中止（`KeyAction::Cancel` → `cancel.cancel()`，`run_app` 即刻 `running=false`）。每个新 turn 由 `start_turn` 先发 `UiCmd::ResetCancel(新 token)` 再发工作命令（mpsc FIFO，4 个派发点：Submit idle / SwitchAndStart / Compact / TurnDone 续跑），刷新 worker 的 `sess.cancel`——否则 token 永久取消会使 `run_loop` 顶部 `is_cancelled()` 永真、后续提交被静默丢弃。
- **弹窗锚点**：`/` 命令面板（`command.rs`）与 `/model` 配置（`model_menu/`）以 composer 顶边 `y` 为底锚渲染下拉浮层（非屏幕居中）。`/model` 内 Enter = 确认当前值并推进下一字段，连续 Enter 到 `[Save]` 提交；`↑/↓` 在文本字段间导航、在 Reasoning/Threshold 上改值。

## 依赖与接口
- 依赖：ratatui 0.29、crossterm、tokio、opencode-session、opencode-core、opencode-store、opencode-llm（estimate）。
- 被依赖：binary crate（`src/main.rs` → `opencode_tui::run_tui`）。
- worker 通道：`UiCmd::{Prompt,SwitchAgent,SwitchAndStart,Compact,SetSkill,ReloadConfig,ResetCancel,Quit}` → `UiEvent::{Session(SessionEvent),TurnDone}`。`ResetCancel(CancellationToken)` 在每个 turn 开始前由 `start_turn` 发出，把 `sess.cancel` 换成未取消的新 token（loop 的 `cancel` 句柄同步重指），保证双击 Esc 中止后仍可提交。

## 相关模块
- [agents/session](../session/index.md) — SessionEvent 来源。
- [agents/core](../core/index.md) — ChatView 依赖 Message/ContentBlock。

## 代表性锚点
- ChatBlock/Subagent ctx 视图测试：`chat::tests::subagent_events_render`
- Kitty 键盘协议 Ctrl+D/Ctrl+C 退出回归：`app::tests::kitty_ctrl_d_quits` / `app::tests::kitty_ctrl_c_quits`
- thinking 命中测试一致性：`chat::tests::thinking_headers_match_flatten_line_indices`
- worker cancel-token 交换回归：`worker::tests::rebind_session_swaps_the_active_cancel_token`
- cancel-token 刷新回归（双击 Esc 后可提交）：`worker::tests::reset_cancel_replaces_with_fresh_uncancelled_token`、`session/tests/cancel_reset.rs`
