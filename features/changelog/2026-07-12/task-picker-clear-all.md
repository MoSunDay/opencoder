Commit: (working-tree, pre-initial-commit)

# `/task` 列表「一键清空所有任务记录」+ 二次确认

## 背景

`/task` 弹窗（`crates/tui/src/task.rs` 的 `TaskPicker`）只能切换 / 新建 / 恢复会话，历史任务记录会无限累积，无法在界面内清理。用户希望在激活 `/task` 列表后有一键清空入口，且因属不可逆操作需二次确认。

## 变更

### 1. Store 层新增批量删除接缝
- `Store` trait 新增 `clear_other_sessions(&self, keep_session_id: &str) -> Result<u64>`（`crates/store/src/store.rs`）——删除除当前活动会话外的所有 session，返回删除条数。
- libsql 实现 `sessions::clear_others`（`crates/store/src/libsql_store/sessions.rs`）：单条 `DELETE FROM sessions WHERE id != ?`；子表（messages / session_inputs / session_events / subagent_tasks）由 schema 的 `ON DELETE CASCADE` 外键自动级联清除，无需逐表删。

### 2. `TaskPicker` 加 destructive 行 + 两步确认
- `TaskPicker` 新增字段 `current_session_id`（构造时由 app 传入，始终豁免删除并在列表标 `(current)`）与 `confirm_clear`（两步确认 guard）。
- 列表底部新增红色 `✕ Clear all N task(s)` 行（仅当存在可删会话时出现；N = 可删数，排除当前会话）。
- 键盘语义（`handle_task_key`）：
  - 选中 Clear-all 行 + Enter → 进入确认态（标题切换为红色 `⚠ Clear ALL N task(s)? Enter=confirm, Esc=cancel`），不执行删除。
  - 确认态下第二个 Enter → 发出 `TaskOutcome::ClearAll { keep_session_id }`；Esc 仅取消确认、picker 保持打开；↑/↓ 在确认态锁定；Ctrl+C/D 仍即时退出。
- `TaskOutcome` 新增 `ClearAll { keep_session_id }` 变体（加 `#[derive(Debug)]` 便于测试断言）。

### 3. app 层接线（`crates/tui/src/app.rs`）
- 构建 picker 时传入 `session_id.clone()`（当前活动会话 id）。
- `TaskOutcome::ClearAll` 分支：调 `store.clear_other_sessions(&keep)`，成功则用刷新后的 `list_sessions` 原地 `reset_sessions`（picker 保持打开，用户直接看到精简后的列表）并推绿色 marker `[/task] cleared N of M task(s)`；失败则 `reset_confirmation` + 红色错误 marker。

## 安全约束
- **当前活动会话豁免**：删除时 `WHERE id != keep`，运行中的 worker / 当前对话不受影响——即便清空进行中也可安全使用。
- **运行中任务不清空（idle gate）**：`TaskOutcome::ClearAll` 在 app 层经 `gate_clear_all(running)` 守卫——`running == true`（一个 turn / subagent 在飞行中，子会话仍在被写入）时**拒绝清空**，仅重置确认态并推黄色 busy marker（`[task] clear busy — retry when idle`）；只有 idle（所有 subagent 已返回）才走真实删除。这避免运行中 subagent 的 `sub-<id>` 子会话被中途删除导致下一次 `append_message` FK 违约。与 `/compact` 的 `gate_compact` 同构。
- 子表经外键级联删除，不会留下孤儿 messages/inputs/events/subagent_tasks。
- 空 / 仅当前会话：Clear-all 行自动隐藏，无可删项时无入口。
- DB 错误：picker 不关闭、确认态重置、红色 marker 提示，列表不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 仅当前会话时无 Clear-all 行 | `clear_row_hidden_when_nothing_deletable` | `crates/tui/src/task.rs`（新增） |
| 存在其它会话时出现 Clear-all 行 + 计数 | `clear_row_shown_when_other_sessions_exist` | `crates/tui/src/task.rs`（新增） |
| 首个 Enter 仅进入确认态（不删除） | `first_enter_on_clear_row_arms_confirmation` | `crates/tui/src/task.rs`（新增） |
| 第二个 Enter 发出 `ClearAll { keep }` | `second_enter_emits_clear_all_with_keep` | `crates/tui/src/task.rs`（新增） |
| Esc 取消确认但保留 picker | `esc_cancels_confirmation_but_keeps_picker_open` | `crates/tui/src/task.rs`（新增） |
| 确认态下 ↑/↓ 锁定 | `navigation_locked_during_confirmation` | `crates/tui/src/task.rs`（新增） |
| 确认态下 Ctrl+C 仍退出 | `ctrl_c_quits_even_during_confirmation` | `crates/tui/src/task.rs`（新增） |
| 批量删除保留当前、级联清子表、幂等 | `clear_other_sessions_keeps_current_and_cascades` | `crates/store/tests/store_integration.rs`（新增） |
| idle（无 subagent 飞行）允许清空 | `gate_clear_all_runs_when_idle` | `crates/tui/src/worker.rs`（新增） |
| 运行中（turn/subagent 飞行）拒绝清空 | `gate_clear_all_rejects_when_running` | `crates/tui/src/worker.rs`（新增） |

- 全量回归：`cargo test --workspace` → 全绿（tui 130 unit 含新增 9：7 picker + 2 gate；store 含新增 1）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 行数：`task.rs` 411 ≤ 800（迭代中）；`app.rs` 782 ≤ 800；`worker.rs` ~+22。

## Impact Surface

- **TUI 用户**：`/task` 列表底部多出红色 `✕ Clear all N task(s)`；选中 Enter → 红色确认提示 → 再 Enter 执行（保留当前对话）；Esc 随时取消。
- **当前会话 / 运行中的 worker**：不受影响（豁免删除）。
- **不影响** CLI / Web / session / llm —— 仅 store trait 增一方法 + tui `/task` picker。

## Related Docs
- [agents/tui](../../agents/tui/index.md)（已同步：`/task` picker 新增 clear-all 行 + 两步确认）
- [agents/store](../../agents/store/index.md)（已同步：`Store::clear_other_sessions` 接缝）
