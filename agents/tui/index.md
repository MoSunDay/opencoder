Commit: 363c826e2d66608bab94689112d51f27efa800a8

# tui 模块

ratatui + crossterm 交互界面。细节以代码为准。

## 索引

- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环
- `src/worker.rs` — worker actor 持 SessionState，事件桥接 UI 通道
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射（模式切换门禁）
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染、渲染入口
- `src/agent_menu.rs` — `/agent` 自定义 Agent 选择器
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- `src/ts_mirror.rs` — tmux 会话冷启动恢复
- `tests/` — 集成测试（agent_menu_catalog / agent_mention_flow /
  agent_switch_persist / bootstrap_agent_override 等）

## 边界

- 不持有 SessionState（worker 持有）；notepad/本地 `!cmd` 不进模型 context。

相关模块：[core](../core/index.md)、[session](../session/index.md)。
