# TUI 子任务取消/中断结算、观察器失败隔离与切换暂存臂收敛

一句话：`/subtasks` 的 `c` 取消改发 Server `cancel` 并把 `interrupted` 在本地
结算为终态；观察器传输失败按目标隔离，单个目标连续 5 次失败不再拖累健康
目标；会话切换统一收敛 `arm_after_switch`，失败不再遗留陈旧父指针。

## 背景与结论

三项修复都在 TUI 子任务链路，互不改变 Server 执行契约、存储 schema 与配置格式。

### BUG-1：取消动作错误，`interrupted` 永远 pending

旧行为：

- `/subtasks` 的取消（`c`）调用 `remote::subtasks::cancel`，实际发送
  `{"action":"interrupt"}`；`interrupt` 是 Server 的中断控制命令（落
  `Interrupted`），并不等价于取消（落 `Cancelled`），取消动作因此没有按
  预期落到取消语义。
- `subtask_registry::settle_status` 把 `ExecutionStatus::Interrupted` 映射为
  `None`，watcher 认为该执行仍可推进；`is_terminal` 也不包含
  `"interrupted"`，`/subtasks` 的完成计数与 pending 列表一直保留它，
  Ctrl+Enter 追加被 `subtask 尚未完成，完成后再追加` 门禁拦下。

根因：取消按钮的 action 用错；TUI 本地终态口径漏了 `Interrupted`。

最小修复：

- `cancel()` 改发 `{"action":"cancel"}`。
- `settle_status(ExecutionStatus::Interrupted)` 返回 `Some("interrupted")`，
  注册表记录为 `"interrupted"`；`is_terminal` 增加 `"interrupted"`，pending
  列表与完成计数一并按终态处理。
- Ctrl+Enter 追加门禁继续只问 `is_terminal`；`interrupted` 由此与
  done/error/cancelled 一样可追加。结果无摘要时 `append_text` 的 `(未完成)`
  兜底保持不变。
- Server 侧 `ExecutionStatus::terminal()` 仍只认
  `Done | Error | Cancelled`，控制面与取消命令实现均未改动；`"interrupted"`
  只是 TUI 注册表/展示层的本地终态口径。

### BUG-2：单个目标失败拖垮全部剩余项

旧行为：watcher 用全局 `round_error` 与轮级 `failed_rounds`。任意目标一次
传输错误就把整轮标记为失败；连续 5 轮后把当时所有 remaining target 一次性
按 `error` 结算并退出。只要坏目标与健康目标同批，健康目标也会被结算为
`error`，即使它每次轮询都成功；反过来，某目标的健康轮也不会清掉别的目标
留下的全局错误，失败计数无法按目标归零。

根因：失败状态是**轮级/全局**的，结算也是**批量**的，没有目标级归属。

最小修复：

- 每个目标持有独立 `TargetState { target, failed_rounds, last_error }`；每轮
  独立轮询每个仍未终结的目标。
- 删除全局 `round_error` 与批量结算：某目标 `failed_rounds >= 5` 时只发送
  该目标自己的 `error`（带该目标最后一次错误摘要）并移出轮询；其他目标
  不受影响。
- 任一成功轮询（`fetch_status` 返回 `Ok`，含仍非终态的状态）把该目标
  `failed_rounds = 0`、`last_error = None`，失败计数按目标连续计算。
- 抽出 `run_with(interval)` 供测试注入短周期；生产 `run` 仍固定 5 秒轮询。
- UI 通道关闭时立即返回，不为剩余目标补发结算。

### BUG-3：切换失败遗留 `arm_after_switch`

旧行为：`/subtasks` 的 `Enter` 把父会话 id 暂存到 `arm_after_switch`，等待
下一次 session switch 落地。`app.rs` 成功分支先清 `main_task_id` 再 `take`
暂存臂；但 `Err` 分支只 flash 错误，**不清暂存臂**。之后任意一次无关的成功
切换都会把这个陈旧父指针当成刚 staged 的 descent 目标，Ctrl+Enter 追加
可能指向错误父会话。

根因：暂存臂的消费只写在成功分支，失败路径漏了清理。

最小修复：

- `SubtaskUi::settle_switch(bool)` 无条件 `take()` 暂存臂：`true` 时把
  `main_task_id` 设为 staged parent（staged 为 `None` 即清空）；`false` 时
  保留当前 `main_task_id`，只丢弃暂存臂。
- `app.rs` 成功分支调 `settle_switch(true)`，`Err` 分支在 flash 前调
  `settle_switch(false)`；两分支不再各自拼装指针逻辑。

## 变更清单

- `crates/tui/src/remote/subtasks.rs`、`crates/tui/src/subtask_detail.rs` —
  取消命令改 `cancel`，注释同步。
- `crates/tui/src/subtask_registry.rs` — `Interrupted` 映射与 `is_terminal`
  纳入 `interrupted`。
- `crates/tui/src/subtask_watcher.rs` — 目标级 `TargetState`、失败隔离与
  `run_with(interval)`。
- `crates/tui/src/subtask_events.rs`、`crates/tui/src/app.rs` —
  `settle_switch(bool)` 与成功/失败分支。
- `crates/tui/src/app_subtask.rs`、
  `crates/tui/src/app_loop_actions_subtask_tests.rs`、
  `crates/tui/src/remote/tests/subtasks.rs`、
  `crates/tui/src/remote/tests/support.rs` — 终态折叠/flash、追加门禁、
  观察器脚本化状态等测试。
- `crates/core/src/harness/mod.rs` — `SubtaskRef.status` 注释补
  `"interrupted"`（仅注释）。
- `agents/frontend/tui/index.md` — 同步 watcher 失败隔离与 `interrupted`
  本地终态口径。
- 本 changelog。

## 功能 → 测试名 → 文件

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 取消命令 | `cancel_subtask_uses_cancel_command` | `crates/tui/src/remote/tests/subtasks.rs` |
| interrupted 映射 | `settle_status_maps_server_lifecycle_to_registry_status` | `crates/tui/src/subtask_registry.rs` |
| 终态计数 / pending | `counts_and_pending_ids_track_terminal_state` | `crates/tui/src/subtask_registry.rs` |
| watcher 结算 interrupted | `watcher_settles_interrupted_status_once` | `crates/tui/src/remote/tests/subtasks.rs` |
| tick 折叠终态并 flash | `tick_folds_terminal_update_and_flashes` | `crates/tui/src/app_subtask.rs` |
| Ctrl+Enter 追加门禁 | `interrupted_entry_appends_like_other_terminal_states` | `crates/tui/src/app_loop_actions_subtask_tests.rs` |
| 单目标失败隔离 | `watcher_failing_target_does_not_error_healthy_pending_target` | `crates/tui/src/remote/tests/subtasks.rs` |
| 健康轮询清零失败计数 | `watcher_resets_target_failure_streak_on_a_healthy_poll` | `crates/tui/src/remote/tests/subtasks.rs` |
| 切换成功落地暂存父指针 | `switch_landing_adopts_staged_parent` | `crates/tui/src/subtask_events.rs` |
| 切换失败丢弃暂存臂 | `switch_error_drops_staged_arm_and_keeps_current_descent` | `crates/tui/src/subtask_events.rs` |

## 既有回归

以下用例保持通过，用于锁定本次改动未破坏原契约：

- `terminal_subtask_settles_once_with_server_status`
  （`crates/tui/src/remote/tests/subtasks.rs`）
- `watcher_completion_folds_tokens_into_parent_registry_and_chat`
  （同上）
- `watcher_maps_idle_agent_convergence_to_done`
  （同上）
- `settles_all_targets_as_error_after_five_failed_rounds`
  （`crates/tui/src/subtask_watcher.rs`；结算已改为按目标独立进行，两目标
  同时失败时仍各得一次 `error`）
- `exits_immediately_without_targets`
  （`crates/tui/src/subtask_watcher.rs`）

## 实测

定向测试（实际执行，均 0 failed）：

```text
cargo test -p opencoder-tui --lib subtask_registry         # 8 passed
cargo test -p opencoder-tui --lib subtask_watcher          # 2 passed
cargo test -p opencoder-tui --lib remote::tests::subtasks  # 7 passed
cargo test -p opencoder-tui --lib subtask_events::tests    # 7 passed
cargo test -p opencoder-tui --lib app_subtask::tests       # 3 passed
cargo test -p opencoder-tui --lib subtask_append_tests     # 4 passed
```

合计 **31 passed / 0 failed**。

- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` →
  exit 0，零警告。
- 格式化：**不能声称全局 fmt 通过**。`cargo fmt --all -- --check` 退出 1，
  原因是 HEAD 上既有、与本次无关的未格式化文件（约 3,613 行 diff；例如
  `crates/agent/src/host/service.rs`、`crates/control/...`）。本次触及的
  10 个 TUI 文件均通过
  `rustfmt --edition 2021 --check --config skip_children=true`；
  `crates/core/src/harness/mod.rs` 只改了一行注释，其 rustfmt 差异位于既有
  测试代码约 249/256 行，不在本次改动行。
- 行数上限：`crates/tui/src/app.rs` = 797 行（上限 800）；其余触及文件均
  ≤ 504 行（最大为 `crates/tui/src/remote/tests/subtasks.rs`）。
- **未运行全量测试**：未运行 `cargo test --workspace`、SPA 全量测试或
  全站 UI 验收；仅运行上述定向用例。

## 已知限制（R1–R5）

- **R1**：注册表 `apply_update` 的 load/save 不是事务性的；并发更新可能
  丢写。不在本次范围；只有实际观察到再升级为 store 事务。
- **R2**：Agent/Operator 在 `resource_admission` 排队时，取消只写 receipt
  不落 execution index；watcher 的 `/index` 查询仍会 404，连续 5 次失败后
  按 `error` 而不是 `cancelled` 结算。运行中/已中断执行不受影响；后续可用
  cancel 响应体回填本地终态。
- **R3**：`fetch_tokens` 失败时显示 0；只影响展示与成本折算，watcher 仍按
  status 结算。
- **R4**：`main_task_id=None` 时 Ctrl+Enter 仍静默（既有行为）。
- **R5**：watcher 串行轮询；单个慢目标会延后同轮健康目标的观察，但不会
  把健康目标结算为 `error`；客户端 30s 超时兜底。

## 回滚

- 直接 revert 本次提交；无 schema、Server、配置或依赖迁移。
- 旧代码不把本地 `"interrupted"` 字符串视为终态，历史注册表中的这类条目
  会重新进入 pending 轮询；这只是展示层影响，Server 侧状态不变
  （`interrupted` 仍可由显式 resume 推进）。
- 回滚时删除本 changelog。
