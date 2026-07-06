Commit: (working-tree, pre-initial-commit)

# plan-act-handoff + /compact

## 变更

### plan→act 手动切换自动执行
- **移除 `plan_exit` 工具**：plan agent 不再调用工具切换到 act。计划以纯文本输出，turn 自然结束（`finish_reason: stop`）。删除 `crates/session/src/tools/plan.rs`、tools/mod.rs 注册、agent.rs 工具列表。
- **移除 runner post-tool hook**：`runner.rs` 中 `switched_to_act` 标志 + agent 自动切换 + `plan_to_act_note()` synthetic 注入全部删除。
- **PLAN_SUFFIX 重写**：只读模式说明 + "如有歧义向用户提问确认"对齐指令。不再提及 plan_exit。
- **`SwitchAndStart` UiCmd**：TUI 检测 plan→act 且 `!running` 时发送。worker 切 agent 后以空 prompt 调用 `run_session` → runner 跳过 user message 记录 → 直接进 `run_loop`。系统提示从 plan 变为 act，模型读对话历史中的计划自动开始执行。
- **`plan_to_act_note()` 删除**：不再需要 synthetic transition note。

### /compact 手动压缩命令
- **`SlashAction::Compact`**：`/compact`（或 `/c`）加入 slash command picker。空闲时触发 `UiCmd::Compact` → worker 调用 `compaction::compact(sess, &registry)` 总结早期消息。
- 压缩结果通过 `SessionEvent::Compaction(summary)` 反馈到 TUI。

### worker.rs 模块提取
- `UiCmd`/`UiEvent`/`process_cmd` 从 `app.rs`（813 行）提取到 `worker.rs`（92 行），消除主 worker 与 `/task` worker 的重复 match 臂。`app.rs` 降至 729 行。

## 涉及文件
- `crates/core/src/agent.rs` — plan agent tools 移除 plan_exit；PLAN_SUFFIX 重写
- `crates/session/src/runner.rs` — 删除 plan_exit hook + plan_to_act_note import
- `crates/session/src/prompt.rs` — 删除 `plan_to_act_note()`
- `crates/session/src/tools/mod.rs` — 移除 plan module + PlanExitTool 注册
- `crates/session/src/tools/plan.rs` — 删除
- `crates/tui/src/worker.rs` — 新增：UiCmd + UiEvent + process_cmd
- `crates/tui/src/app.rs` — import worker 模块；SwitchAgent plan→act 检测；/compact dispatch
- `crates/tui/src/command.rs` — SlashAction::Compact + COMMANDS + parse/dispatch
- `crates/tui/src/keybind.rs` — help 文本更新
- `crates/tui/src/lib.rs` — 注册 worker 模块

## 测试
- 移除：`plan_exit_writes_plan_file`（tools_contract.rs）、`plan_to_act_note_mentions_execution`（prompt.rs）
- 新增：`menu_filters_compact`（command.rs）、`/c` 和 `/compact` parse 测试
- 全量：227 passed, 0 failed, clippy --all-targets -D warnings clean

## 修复（review round 2）
- **F1〔P1〕`/task` 切换会话后硬中止失效**：主循环的 `cancel`（`app.rs`）是不可变 `let` 且从未回绑，双击 Esc 取消的是首个会话的废弃 token，切换后的活跃会话无法中断（mid-tool 硬中止 + turn 边界 cancel 均失效）。修复：`let mut cancel`，并把回绑抽成纯函数 `worker::rebind_session`（命令通道 / 事件流 / session_id / cancel 四项一起移交给新会话），于 `/task` 切换处调用。
- **`/compact` busy 反馈**：运行中触发 `/compact` 原先静默无效；现经纯函数 `worker::gate_compact(running) -> CompactGate{Run, SkipRunning}` 路由，`SkipRunning` 时推一条黄色 `[compact] busy — retry when idle` marker。
- 测试（worker.rs 内联）：`gate_compact_runs_when_idle` / `gate_compact_rejects_when_running` / `rebind_session_swaps_the_active_cancel_token`（regression guard：回绑后 cancel 新会话 token、旧 token 或phaned 不受影响）。
- 记忆口径：F1 属接线修复、不改语义模型（cancel 仍由调用方挂载），按 repo-local-memory Final Gate 不落 `agents/*`；仅本 changelog 记录。
