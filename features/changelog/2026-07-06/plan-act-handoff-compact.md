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
