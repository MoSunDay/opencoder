Commit: 2aa44247d199d782881b9ee64921c4c6de2e6199

# tui 模块

ratatui + crossterm 交互界面。细节以代码为准。

## 索引

- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环；`src/app_bootstrap.rs` 读取启动时的 Harness/env 并恢复固定的会话运行态，`src/app_task.rs` 让 `/task` 新会话继承启动选择
- `src/worker.rs` — worker actor 持 SessionState，事件桥接 UI 通道
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射（模式切换门禁）
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染、渲染入口
- `src/model_menu/` — `/config` 表单包含 `local-memory` 开关，写入顶层 `local_memory` 配置。
- [agent_menu.rs](../../crates/tui/src/agent_menu.rs) — 自定义卡选择器实现保留；TUI 不展示 Agent 命令，手动提交 `/agent` 或 `/agents` 会被拦截。
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- `src/ts_mirror.rs` — tmux 会话冷启动恢复
- `src/hooks.rs` — 从 `~/.opencoder/hooks.json` 读取 TUI 事件命令并异步执行；
  `app_loop.rs` 在 `question` 工具开始和最终 `TurnDone` 空闲边界触发
- `tests/` — 集成测试（agent_menu_catalog / agent_mention_flow /
  agent_switch_persist / bootstrap_agent_override 等）

## 边界

- 不持有 SessionState（worker 持有）；notepad/本地 `!cmd` 不进模型 context。
- TUI 将 `--wrap`、`--envs` 交给共享 `SessionState`；Codex 的执行、事件解码和进程回收由 [session](../session/index.md) 负责，TUI 不维护另一套适配器。
- `/act`、`/plan` 通过 worker 切换和持久化，独立于菜单目录；恢复与无额外消息约束见
  [agent_switch_persist.rs](../../crates/tui/tests/agent_switch_persist.rs)。

相关模块：[core](../core/index.md)、[session](../session/index.md)。
