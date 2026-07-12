Commit: (working-tree, pre-initial-commit)

# tui 模块

## 职责
ratatui + crossterm 交互界面。3-region 布局、事件循环、鼠标命中测试、键盘分发、subagent 可折叠 + ctx 切换。

## 边界与非目标
- 不直接调 Store CRUD（经 worker `UiCmd` 通道）；不持有 SessionState（worker 持有）。
- 非目标：TUI 不是 Web 替代品——无 SSE replay，仅 live event 流。

## 关键抽象
- `ChatView`（`src/chat.rs`）：`blocks: Vec<ChatBlock>` + `agent/status/subagents_running/subagents_total/context_used`。`apply(&SessionEvent)` 逐事件更新——核心接缝；内部 `track_context` 只累加本 view transcript token（不递归 SubagentChild——父 view 的 context_used 仅含自身，子 view 独立维护自身子树）。app.rs 不再有独立 `context_used` 变量——直接读写 `chat.context_used`。
- `ChatBlock`（`src/chat.rs`）：`Marker(Vec<Line>)` / `Assistant{raw,rendered,done}` / `Thinking{text,collapsed}` / `Tool{header,output}` / `Subagent{id,child_session_id,kind,prompt,view:ChatView,done,ok,summary}`。`flatten()` → `Vec<Line>` 供 Paragraph 渲染。
- `Subagent` block 不可内联展开：始终单行表头 `⇲ subagent [kind] prompt [●/✔/✘ status, N tools] [→ view]`（running=黄● / done=绿✔ / failed=红✘）。点击表头直接进入子会话 ctx 视图（body 渲染子 ChatView + 子 view 自有 context_used 驱动状态栏 ctx 仪表），Esc 返回主进程。
- `MouseHits`（`src/render.rs`）：每帧重算的命中目标——`jump_btn/body/queue_btns/thinking_btns/subagent_btns`。`record_thinking_hits` / `record_subagent_hits` 映射逻辑行号到屏幕行号。
- `key_handler`（`src/key_handler.rs`）：`KeyAction` enum + `handle_key()` 分发器 + `move_hist()` 历史导航。从 app.rs 提取（行数控制）。Ctrl+C/Ctrl+D 在 CONTROL 块同时匹配 `Char('c')|Char('d')` 和 Kitty 键盘协议下的原始控制字符 `Char('\u{3}')|Char('\u{4}')`——后者由 `DISAMBIGUATE_ESCAPE_CODES` 导致，不带 'c'/'d' 字面量。Ctrl+A/Ctrl+E = 光标跳首/尾（char 安全，与 Home/End 一致）。`$` 在输入任意位置触发 skill picker（不再限空输入）；picker 选中插入 `{$name}` token 到光标，提交时由 `apply_skill_tokens` 解析。
- `app_helpers`（`src/app_helpers.rs`）：从 app.rs 提取的 `pub(crate)` 自由函数——`mk_input`/`start_turn`/`worker_dead`/`sys_tokens_for`/`push_user`/`data_dir_for`/`pre_key_intercept`/`handle_mouse`/`apply_skill_tokens`/`resolve_and_warn`。经 `pub(crate) use crate::app_helpers::{...}` 在 app.rs 重导出，`crate::app::*` 测试引用路径不变。`start_turn` 返回 `bool`（`false` = 命令通道关闭 = worker 已死，调用方推 `[worker stopped]` marker + break）。`pre_key_intercept` 处理 Esc-subagent 退出 + Ctrl+L（折叠所有 thinking / 退出 subagent / 清空输入）。`handle_mouse` 处理全部鼠标事件（点击/拖拽/滚轮/选择）。`apply_skill_tokens` 解析内联 `{$name}` token 并激活 skill（写入 `skill_handle: Arc<Mutex<Option<String>>>` = `session.skill_prompt`）。`resolve_and_warn` 包装之并推未解析 skill 警告 marker。
- `skill_token`（`src/skill_token.rs`）：纯函数 `extract_skill_tokens(text) -> (String, Vec<String>)`——剥离 `{$name}` token 返回干净文本 + 有序名称列表。零依赖、UTF-8 安全。`ChatView::collapse_all_thinking()`（`src/chat.rs`）一键折叠所有 Thinking 块（Ctrl+L 绑定）。
- `selection`（`src/selection.rs`）：鼠标拖拽文本选择 + OSC52 剪贴板复制。选择以绝对内容行 `[a,b]`（`screen_row + scroll`）追踪——滚动时锚定文本不漂移；松开鼠标 `finish_copy` 提取选中逻辑行（整行取，v1 行范围模型）经 `copy_osc52`（vendored RFC-4648 base64 + `ESC]52;c;` 序列，SSH 可用、无外部依赖、失败静默）写入系统剪贴板。`render_overlay` 在 Paragraph 之上叠反白高亮（fg/bg 互换）。测试文件拆分（`#[path]` include）：`app.rs`→`app_tests.rs`、`chat.rs`→`chat_tests.rs` + `subagent_tests.rs`、`render.rs`→`render_tests.rs`，各经 `#[cfg(test)] #[path = "..."] mod tests;` 引入——模块路径不变（`app::tests`/`chat::tests`/`chat::subagent_tests`/`render::tests`），仅为控制单文件 ≤800 行迭代上限，不影响公开 API。
- 手动 `draw_scrollbar()`（`src/render.rs`）：替代 ratatui ScrollbarState——简单比例 `scroll_y * max_off / max_scroll`，短内容对齐正确。滚轮处理器换行宽度 `r.width - 3` 与 render 的 `text_w = inner.width - 1` 对齐。
- **composer 与状态栏渲染**（`src/render.rs`）：`render_composer` 按行拆分输入（`split('\n')`），首行带 `❯ ` Cyan 提示符，用 `Paragraph::wrap(Wrap{trim:false})` 软换行 + `.scroll()` 支持滚动；`render_status` 底栏显示 model | [agent] | dir | ctx%（**不再含 "opencoder" 品牌字样**）；`place_cursor` 经 `composer::cursor_row_col`（同时处理显式 `\n` 和软换行）计算 (row,col) 后 `f.set_cursor_position` 定位可见光标。render 测试首次引入 ratatui `TestBackend`（in-process buffer 断言 + cursor position 验证）。

## 主流程
`run_app` loop：每帧 `render()` → `tokio::select!` 三臂（终端事件 `input_rx` / worker `evt_rx` / anim ticker）。**输入采集**（`src/input.rs`）：专用 OS 线程跑同步有界 `crossterm::event::poll(150ms)`+`read()`，经 tokio mpsc `blocking_send` 投递到 `input_rx`——弃用 `EventStream`（其 async reader 任务走 mio + tokio waker，一旦停滞 `select!` 输入臂永不触发，整循环冻死、Ctrl+C/D 全失效）；改走同步路径端到端绕过该失败模式（crossterm unix source 用 `filedescriptor::poll`+非阻塞读，有界；`poll` 成功后 `read()` 从内部队列弹已入队事件，不触及 `poll(None)` 回退）。receiver drop（`is_closed()`）即令线程退出。**终端生命周期**（`src/terminal.rs`）：`TerminalGuard` RAII（`run()` 持有）——enter 开 raw+alt-screen+鼠标+Kitty 并装 panic hook（panic 时先恢复终端再打印），Drop 幂等恢复；任何退出路径（正常/`?`错误/panic unwind）都恢复终端，不再"变砖"。`input_rx.recv()` 返回 `None`（采集线程退出/stdin EOF）时 `UiCmd::Quit` + break。

- **render**：`display_chat` / `display_agent` / `display_ctx` / `display_sys` = subagent_focus 时取子 ChatView 及其 `context_used` + 缓存的 `subagent_sys`（进入时 `sys_tokens_for(kind, workdir, None)` 算一次），否则取 `&chat` + `chat.context_used` / `sys_tokens`。`display_agent` = focus 时 `← [Esc] back | ⇲sub [kind] prompt`。
- **Session 事件**：`chat.apply(&sev)` 内部调 `track_context` 更新 `chat.context_used`（只计本 view token，SubagentChild 不计入父），SubagentChild 路由到子 block view（子 view 的 apply 独立更新自身 context_used）；TranscriptReset 重建 ChatView（context_used 归零）。
- **subagent ctx 切换**：`subagent_focus: Option<usize>` 状态。进入时保存 `parent_scroll/parent_follow`，退出时恢复。Esc 优先拦截（在 handle_key 之前）。
- **输入模式**：Enter = steer（运行中）/ submit（idle）；Tab = followup（运行中）/ submit（idle）；Shift+Tab 切 plan/act。
- **中止与续跑**：双击 Esc 硬中止（`KeyAction::Cancel` → `cancel.cancel()`，`run_app` 即刻 `running=false`）。每个新 turn 由 `start_turn` 先发 `UiCmd::ResetCancel(新 token)` 再发工作命令（mpsc FIFO，4 个派发点：Submit idle / SwitchAndStart / Compact / TurnDone 续跑），刷新 worker 的 `sess.cancel`——否则 token 永久取消会使 `run_loop` 顶部 `is_cancelled()` 永真、后续提交被静默丢弃。`start_turn` 返回 `bool`：`false` = 命令通道关闭（worker 已死，panic 或意外退出），派发点收到 `false` 时 `worker_dead()` 推 `[worker stopped]` marker 并 break。输入采集在独立 OS 线程，worker 死后 UI 仍响应 Ctrl+C/D，用户可干净退出而非面对冻死 spinner。
- **弹窗锚点**：`/` 命令面板（`command.rs`）与 `/model` 配置（`model_menu/`）以 composer 顶边 `y` 为底锚渲染下拉浮层（非屏幕居中）。`/model` 内 Enter = 确认当前值并推进下一字段，连续 Enter 到 `[Save]` 提交；`↑/↓` 在文本字段间导航、在 Reasoning/Threshold 上改值。`/task` 会话选择器（`task.rs`）为屏幕居中模态：`+ New task` + 历史 session 列表（当前会话标 `(current)`），底部红色 `✕ Clear all N task(s)` destructive 行——选中 Enter 进入红色两步确认态（再 Enter 触发 `TaskOutcome::ClearAll`；app 层经 `gate_clear_all(running)` 守卫：`running==true` 即 turn/subagent 飞行中时拒绝并推黄色 busy marker，idle 后才调 `Store::clear_other_sessions` 删除除当前会话外的全部会话并刷新列表；Esc 取消确认、确认态锁定 ↑/↓、Ctrl+C/D 仍即时退出）。

## 依赖与接口
- 依赖：ratatui 0.29、crossterm（不再启用 `event-stream` feature——输入采集改专用线程 poll/read）、tokio、opencoder-session、opencoder-core、opencoder-store、opencoder-llm（estimate）。
- 被依赖：binary crate（`src/main.rs` → `opencoder_tui::run_tui`）。
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
- 输入采集线程 receiver drop 即退出：`input::tests::pump_exits_when_receiver_dropped`
- 单按 Esc 在同步 pump 上有界投递（pty 基础交付回归）：`tests/input_pty.rs::lone_esc_is_delivered_within_bound`
- **不完整 CSI（`\x1b[`）不挂死 pump；补全字节有界投递（结构不变量直接证据）**：`tests/input_pty_incomplete.rs::incomplete_csi_does_not_wedge_pump`
- pty 线束（openpty + raw + fd 0 重定向 + Drop 恢复）：`tests/common/mod.rs::PtyStdin`
- 终端恢复幂等（无 TTY 也可调用）：`terminal::tests::restore_is_idempotent_without_a_tty`
- `write_restore` 依序发出三条恢复序列：`terminal::tests::write_restore_emits_all_three_sequences`
- panic hook 先恢复终端再链到原 hook：`terminal::tests::hook_body_restores_before_chaining_to_prev`
- worker 死亡检测（start_turn 通道关闭返 false）：`app::tests::start_turn_reports_false_when_worker_is_dead`
- 状态栏不含 "opencoder" 品牌、仍含 model/agent/dir/ctx：`render::tests::status_bar_omits_branding`
- 状态栏运行态显示 spinner + status 文本：`render::tests::status_bar_running_shows_spinner_and_status`
- composer 多行渲染（❯ 提示符 + 文本 + 跟随/跳转标签）：`render::tests::composer_renders_prompt_and_multiline_text`、`render::tests::composer_jump_label_when_not_following`
- 光标定位（首行/次行/软换行/滚动）：`render::tests::place_cursor_row_zero`、`render::tests::place_cursor_second_line`、`render::tests::place_cursor_soft_wrap_advances_row`、`render::tests::place_cursor_with_scroll`
- cursor_row_col 软换行边界（CJK 宽字符/width=1/空行/char_idx 越界）：`composer::tests::cursor_row_col_soft_wrap_edge_cases`
