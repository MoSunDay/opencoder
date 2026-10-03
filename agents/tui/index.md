Commit: 40dd45ed4c7c3240a0e879ac5bfec391ffb5a03c

# tui 模块

ratatui + crossterm 交互界面。细节以代码为准。

## 索引

- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环；`src/app_bootstrap.rs` 恢复本地运行态或远端绑定，`src/app_task.rs` 管理独立任务及本地新任务的运行设置
- [worker.rs](../../crates/tui/src/worker.rs) — 按远端绑定分派 Server actor 或本地会话 actor，统一桥接 `SessionEvent` 到现有聊天渲染。
- [clipboard.rs](../../crates/tui/src/clipboard.rs)、[app_loop_paste.rs](../../crates/tui/src/app_loop_paste.rs) — 将系统剪贴板图片转为 PNG 附件；`keymap.paste_image` 配置快捷键，终端占用默认组合键时的设置见 [Windows 使用说明](../../docs/windows.md)。
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射（模式切换门禁）
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染、渲染入口
- `src/model_menu/` — `/config` 表单包含 `local-memory` 开关，写入顶层 `local_memory` 配置。
- [agent_menu.rs](../../crates/tui/src/agent_menu.rs)、[remote/selection.rs](../../crates/tui/src/remote/selection.rs) — `/agent` 异步读取 Server 能力库投影；`self` 创建本地空会话，能力 ID 创建独立远端任务。
- [remote/](../../crates/tui/src/remote/mod.rs) — Server HTTP、SSE 重连、历史恢复、问题转发与任务绑定；远端执行不进入本地模型循环。
- [app_task.rs](../../crates/tui/src/app_task.rs)、[task.rs](../../crates/tui/src/task.rs) — `/task` 混合列出本地任务与远端书签；切换替换消息、用量、问题和事件通道，远端旧任务只断开连接。
- [key_handler.rs](../../crates/tui/src/key_handler.rs)、[app_submit.rs](../../crates/tui/src/app_submit.rs) — `@` 按原文提交；远端用户消息由 Server 消费事件显示，避免重复回显。
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- [windows_console.rs](../../crates/tui/src/windows_console.rs) — 保存 Windows 控制台模式、启用 VT 输出，并在正常退出、panic 和控制台关闭事件中恢复。
- `src/ts_mirror.rs` — tmux 会话冷启动恢复
- `src/hooks.rs` — 从 `~/.opencoder/hooks.json` 读取 TUI 事件命令（宿主 Bash / PowerShell 7、3 秒超时、
  失败仅 debug 日志）异步执行；`turn_done` 仅在 `app_loop.rs` 的最终空闲分支触发——
  drain 重启（`drain_pending`）与用户取消（`cancelled`）路径不发射；`question` 仅在
  live `question` ToolStart 触发，store replay 不触发
- `tests/` — 集成测试（agent_menu_catalog / agent_mention_flow /
  agent_switch_persist / bootstrap_agent_override 等）

## 边界

- 不持有 SessionState（worker 持有）；notepad/本地 `!cmd` 不进模型 context。
- 本地任务将 `--wrap`、`--envs` 交给共享 `SessionState`；Codex 执行与事件解码由 [session](../session/index.md) 负责。远端任务使用 Server 注册的执行器、模型与环境。
- [remote/session.rs](../../crates/tui/src/remote/session.rs) 将远端绑定保存在已有 Harness 运行态中；书签缺失或不一致时拒绝恢复。凭据只由连接层读取 `OPENCODER_SERVER_TOKEN`，不保存到书签。
- [remote/worker.rs](../../crates/tui/src/remote/worker.rs) 使用同一执行 ID 提交首轮与后续输入；退出、切换和 actor 释放只断开连接，显式取消发送 Server `interrupt`。
- [remote/transcript.rs](../../crates/tui/src/remote/transcript.rs) 恢复有界消息块与事件，按消费事件确定用户消息边界；压缩后恢复未结束回合的增量，保留用量与展示原文。
- `/act`、`/plan` 通过 worker 切换和持久化，独立于菜单目录；恢复与无额外消息约束见
  [agent_switch_persist.rs](../../crates/tui/tests/agent_switch_persist.rs)。

相关模块：[core](../core/index.md)、[session](../session/index.md)、[control](../control/index.md)、[web](../web/index.md)。行为规则见 [Agent 调度平台](../../features/agent-platform/index.md#tui-任务入口)。
