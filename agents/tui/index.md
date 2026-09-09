Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# tui 模块

ratatui + crossterm 交互界面。

## 关键路径
- `src/lib.rs` — `TuiOpts::with_harness` 承接 CLI Harness/环境；`fresh_agent_name` 默认链
- `src/app.rs`、`src/app_loop.rs` — App 状态与主事件循环、命令分发
- `src/key_handler.rs`、`src/keymap.rs` — 键盘分发与键位映射
- `src/worker.rs` — worker actor 持 SessionState；`spawn_ui_event_forwarder` 容量 512、`DELTA_MIN_CAPACITY=64` 时 shed TextDelta
- `src/command.rs` — `SlashAction` 斜杠命令枚举（Notepad/Sidecar/Ap/Mcp/Envs/Cli/Skill/Ps/Stop 等）
- `src/composer.rs` — 输入框；`wrap_rows` 可视行布局
- `src/chat.rs`、`src/chat_steps.rs`、`src/chat_headers.rs`、`src/chat_step_render.rs` — 消息渲染，测试在 `src/chat_tests/`
- `src/render.rs` — 渲染入口；`notepad: Option<&NotepadView>` 全屏分支
- `src/notepad/` — 全屏文件查看器：`tree.rs` 文件树 + `editor.rs` vim 编辑器（`editor_layout.rs` 软换行、`search.rs` rg/grep、`keys.rs`）
- `src/vim/` — vim 引擎（normal/insert/command/motion/undo）
- `src/bash_exec.rs` — 主 composer `!` 前缀触发的本地 bash
- `src/queue_admitter.rs`、`src/steer_admit.rs` — 队列 admit 与键盘 steer 离环 actor
- `src/subagent_input.rs` — subagent 输入（steer 保持内联）
- `src/app_bootstrap.rs` — onboarding 与 fresh 路径 agent 解析
- `src/ts_mirror.rs` — `TsMirrorStore` tmux 会话冷启动恢复
- `src/session_ui/` — 会话切换/replay 渲染
- `src/app_notepad.rs` — notepad 生命周期与本地 bash 结果 poll
- 集成测试：`tests/`（notepad 流程、queue_admit_offloop、question_flow、resume 等）

## 边界
- 不持有 SessionState（worker 持有）；Store 交互经 worker `UiCmd` 通道或离环 actor，不在事件循环内联等写锁。
- notepad、sidecar、ps/stop、本地 `!cmd` 不进模型 context。
- 已有块（Thinking/Turn/Step/call 等）的展开/收起状态仅用户改变；新事件只更新内容。
- 无单独 Codex 渲染器，消费共享消息/事件；Harness 运行契约在 session 模块。

## 相关
- [agents/session](../session/index.md) — 会话运行时契约
- [agents/local](../local/index.md) — tui/ts 子命令入口
- [Agent Harness](../../features/harness/index.md)
