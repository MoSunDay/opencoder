Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# tui 模块

ratatui + crossterm 终端交互界面。细节以代码为准。

## 索引

- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环
- [app_bootstrap.rs](../../crates/tui/src/app_bootstrap.rs) — 恢复运行态或远端绑定
- [app_task.rs](../../crates/tui/src/app_task.rs) — 独立任务管理与本地新任务运行设置
- [worker.rs](../../crates/tui/src/worker.rs) — 分派 Server 或本地会话 actor，
  桥接 `SessionEvent` 到聊天渲染
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射（模式切换门禁）
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染与渲染入口
- [chat_step_live.rs](../../crates/tui/src/chat_step_live.rs) — Step 阶梯的
  流式自动披露：运行位自动展开、Say/结束收起、用户点击优先（与 SPA
  `steps/useLadderDisclosure.js` 同契约）
- [chat_steps_replay.rs](../../crates/tui/src/chat_steps_replay.rs) — 回放侧
  Step 阶梯归一：`coalesce_steps` 折叠 Thinking、按 Say 闭合并组（与
  `chat_steps.rs` 共用阶梯语义）
- `session_ui/replay.rs` — 压缩摘要合成消息（标记来自 session
  `SUMMARY_PREFIX`）由回放入口拦截并延迟渲染为 `Compaction` 折叠块：
  排在保留的最近交互之后、继续执行之前（`tail + summary + 续跑`），
  与发给 LLM 的 `[summary, tail...]` 列表顺序解耦；其余合成消息仍跳过；
  `compaction_block.rs` 的 `finalize_compaction` 对回放孪生块幂等去重
- `src/model_menu/` — `/config` 表单（含 `local-memory` 开关）
- [agent_menu.rs](../../crates/tui/src/agent_menu.rs)、
  [remote/selection.rs](../../crates/tui/src/remote/selection.rs) — `/agent` 菜单
- [remote/](../../crates/tui/src/remote/mod.rs) — Server HTTP/SSE、历史恢复与任务绑定
- [remote/transcript.rs](../../crates/tui/src/remote/transcript.rs) — 远端消息恢复
- [remote/subtasks.rs](../../crates/tui/src/remote/subtasks.rs) — `@capability` 派发
- [task.rs](../../crates/tui/src/task.rs) — `/task` 列表；`src/app_submit.rs` 提交
- [control_helpers.rs](../../crates/tui/src/control_helpers.rs) — 复合控制命令组装
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- `src/ts_mirror.rs` — tmux 会话冷启动恢复
- [hooks.rs](../../crates/tui/src/hooks.rs) — `~/.opencoder/hooks.json` TUI 事件命令
- [clipboard.rs](../../crates/tui/src/clipboard.rs)、
  [app_loop_paste.rs](../../crates/tui/src/app_loop_paste.rs) — 剪贴板图片转 PNG 附件
- [windows_console.rs](../../crates/tui/src/windows_console.rs) — 控制台模式与 VT 输出
- `tests/` — 集成测试（agent_menu_catalog / question_flow / notepad_* 等）

## 边界

- 不持有 SessionState（worker 持有）；notepad 与本地 `!cmd` 不进模型 context。
- 本地任务复用共享 `SessionState`，Codex 执行与事件解码见
  [session](../session/index.md)；远端任务使用 Server 注册的执行器。
- 远端绑定保存在 Harness 运行态；凭据仅由连接层读取 `OPENCODER_SERVER_TOKEN`。

相关模块：[core](../core/index.md)、[session](../session/index.md)、
[control](../control/index.md)、[web](../web/index.md)。行为规则见
[Agent 调度平台](../../features/agent-platform/index.md#tui-任务入口)。
