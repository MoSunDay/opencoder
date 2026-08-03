Commit: (working-tree, pre-initial-commit)

# bugfix sweep: 8 运行时逻辑 / 并发 / 契约缺陷修复

## 背景

本轮对全 workspace 做了一次 runtime-defect 扫描，确认并修复 8 个潜伏缺陷
（Tier-1 #1–#6, Tier-2 #7–#8）。所有修复均遵循纯函数式原则、附回归测试。

基线：1672 tests / 0 failed / 0 clippy warnings。
修复后：1685 tests / 0 failed / 0 clippy warnings（本轮 bugfix +12 新测试；另含并发 TUI 提交 compaction_delta_is_ignored +1）。

## 变更

### Bug #1 — SSE replay 订阅/查询竞态（web）
**文件**: `crates/web/src/api.rs::get_events`

原实现先查询历史事件再订阅广播，存在 TOCTOU 窗口：查询到订阅之间的新事件会丢失。
改为**先订阅再查询**，用 `Arc<Mutex<HashSet<(String,String)>>>` 指纹集对
（seq, kind）去重，确保每个事件恰好投递一次。

### Bug #2 — `created_at` 时间单位错误（cli）
**文件**: `crates/cli/src/ts/actions.rs:149`

`ts` 子命令写 tool-call 记录时用 `now_secs()`（秒级 epoch）填充 `created_at`，
但其他路径均用 `now_ms()`（毫秒）。下游按 ms 解析会得到 1970 年的时间戳。
改为 `now_ms()` 并修正 import。

### Bug #3 — steer 提升 TOCTOU（session）
**文件**: `crates/session/src/runner/steer.rs`

原实现用 `.zip()` 按索引配对 `messages` 与 `steers`，当消息列表在配对前发生变更
（并发追加）会导致错位。抽取纯函数 `match_promoted()`，按 **seq 恒等**匹配
而非索引位置，消除 TOCTOU。

### Bug #4 — `create_session` 吞掉错误（web）
**文件**: `crates/web/src/api.rs`

`create_session` 在 `ensure_session_row` 失败时静默返回 200，客户端误以为创建成功。
改为：`ensure_session_row` 返回 `Result<(), String>`，`create_session` 将错误
以 500 传播。

### Bug #5 — doom-loop / tool-failure 返回 Ok(())（session）
**文件**: `crates/session/src/runner/mod.rs`

`run_loop` 在 doom-loop 守卫触发或工具连续失败时返回 `Ok(())`，调用方无法区分
正常结束与异常中止。改为返回 `Err(anyhow!(...))`，使 web drain（`warn!` 日志）
和 TUI worker（Error 事件 + TurnDone）能正确处理。**契约变更**：调用方已适配。

### Bug #6 — subagent_tasks UPDATE 无状态守卫（store）
**文件**: `crates/store/src/libsql_store/subagent_tasks.rs`

COMPLETE 的 UPDATE 仅 `WHERE task_id=?`，无终态守卫。晚到的 complete 可把
已终态（Completed/Failed）的任务重新覆写（result/ok 字段数据丢失）。
COMPLETE UPDATE 加守卫 `AND status IN ('running', 'cancelled')`，
阻止 Completed→Completed / Failed→Completed 等无效翻转。
CANCEL 保持无守卫：execute.rs 超时恢复路径需要从任意终态强制覆写为 Cancelled。

### Bug #7 — JSON 反序列化静默回退（store）
**文件**: `crates/store/src/libsql_store/messages.rs`, `events.rs`

`serde_json::from_str` 失败时直接 `unwrap_or_default()` 返回空集合，数据损坏
无任何日志。改为 `unwrap_or_else(|e| { warn!(...); default })`，便于诊断。

### Bug #8 — `post_interrupt` 无 handle 返回 ok:true（web）
**文件**: `crates/web/src/api.rs::post_interrupt`

当 session 无运行中的 handle 时，`post_interrupt` 返回 `{"ok": true}`，
客户端误以为中断成功。改为返回 `{"ok": false}`。

## 测试覆盖

| # | Bug | 测试名 | 文件 |
|---|-----|--------|------|
| 1 | SSE replay race | `events_subscribe_first_no_loss_no_dup` | `crates/web/tests/bugfix_contracts.rs` |
| 4 | create_session error | `create_session_returns_500_on_store_failure` | `crates/web/tests/bugfix_contracts.rs` |
| 8 | post_interrupt ok:false | `post_interrupt_no_handle_returns_ok_false` | `crates/web/tests/bugfix_contracts.rs` |
| 2 | created_at ms | `now_ms_is_milliseconds` | `crates/cli/src/ts/actions.rs` |
| 3 | steer match_promoted | `match_promoted_*` (3 cases) | `crates/session/src/runner/steer.rs` |
| 5a | doom returns Err | `doom_loop_guard_terminates_act_phase` | `crates/session/tests/autopilot.rs` |
| 5b | doom in autopilot | `doom_loop_in_initial_run_aborts_autopilot` | `crates/session/tests/autopilot.rs` |
| 6a | COMPLETE guard | `complete_does_not_overwrite_completed_terminal_state` | `crates/store/src/libsql_store/subagent_tasks.rs` |
| 6b | resume path | `complete_allows_cancelled_to_completed_resume_path` | `crates/store/src/libsql_store/subagent_tasks.rs` |
| 6c | timeout override | `cancel_can_override_terminal_for_timeout_recovery` | `crates/store/src/libsql_store/subagent_tasks.rs` |

- 全量回归：`cargo test --workspace` → 1685 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets` → 零警告
- build：`cargo build --workspace` → 零错误

## Impact Surface
- **web**: SSE 事件流不再丢事件；session 创建失败正确报 500；interrupt 语义更准确。
- **cli**: `ts` 子命令时间戳与主路径一致（ms）。
- **session**: doom-loop / tool-failure 现以 Err 传播（调用方已适配）；steer 提升不再错位。
- **store**: subagent 终态完整性得到守卫保护；JSON 损坏有日志可查。

## Related Docs
- [agents/web](../../agents/web/index.md)
- [agents/session](../../agents/session/index.md)
- [agents/store](../../agents/store/index.md)
- [agents/cli](../../agents/cli/index.md)
