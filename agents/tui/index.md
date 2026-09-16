Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# tui 模块

ratatui + crossterm 交互界面。细节以代码为准。

## 索引
- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环
- `src/worker.rs` — worker actor 持 SessionState；事件桥接到 UI 通道
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染、渲染入口
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- `src/ts_mirror.rs` — tmux 会话冷启动恢复
- `tests/` — 集成测试

## 边界
- 不持有 SessionState（worker 持有）；notepad/本地 `!cmd` 不进模型 context。
