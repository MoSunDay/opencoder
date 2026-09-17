Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# tui 模块

ratatui + crossterm 交互界面。细节以代码为准。

## 索引
- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环
- `src/worker.rs` — worker actor 持 SessionState；事件桥接到 UI 通道。`UiCmd::Compact`
  成功路径（`Ok(Some)`/`Ok(None)`）以终端 `SessionEvent::Done` 收尾（持久化 +
  实时转发，web `DrainCmd::Compact` 同语义，供 app_loop Done 处理器 resync
  pending Queue/Steer 并 arm `drain_pending`）；`Err` 仅发 Error 不发 Done
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与映射。`is_bare_mode_switch`
  判定裸 act/plan 切换（`SwitchAgent` 且无尾随任务文本）：turn 运行中 Enter/Tab
  返回 `ModeSwitchBlocked`（输入保留）；复合形式 `/plan review` 仍按任务提交。
  Ctrl+T / `/` 菜单在 `dispatch_mode_switch` 同语义拒绝（busy flash，不排队）
- `src/composer.rs`、`src/chat.rs`、`src/render.rs` — 输入、消息渲染、渲染入口
- `src/agent_menu.rs` — `/agent` primary agent 选择器：目录 = builtin primary
  (act/plan/command，排除 workflow) + `list_agents()` 文件卡（描述取
  `agent_description`，回退 "Custom agent <name>"）；fuzzy 过滤与 SPA `@`
  agent 菜单同语义；pick 产出 `/agent <name> `（经 composer 提交走 runner
  控制头），自身不做 I/O
- `src/notepad/` — 全屏文件树 + vim 编辑器
- `src/vim/` — vim 引擎
- `src/ts_mirror.rs` — tmux 会话冷启动恢复
- `tests/` — 集成测试（`agent_mention_flow.rs` 钉 `/agent` 控制头切换 +
  picker 键路径；`bootstrap_agent_override.rs` 钉默认 agent 三级链
  `--agent` > config > "act"，legacy `active` marker 被忽略）

## 边界
- 不持有 SessionState（worker 持有）；notepad/本地 `!cmd` 不进模型 context。
ker 持有）；notepad/本地 `!cmd` 不进模型 context。
