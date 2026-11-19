# TUI 子任务切换意图原子化、取消终态本地回填与无主任务追加提示

一句话：`/agent` 晚到的查询结果不再覆盖 `/subtasks` Enter 下钻；排队取消
（R2）的 Server `cancel` 响应会直接把本地注册表结算为终态，watcher 的
404/`error` 迟到回报不再回写；无主任务时 Ctrl+Enter 给出明确提示而不是
静默（R4）。

## 背景与结论

三项修复都在 TUI 子任务链路，改动只涉及 `crates/tui/`：

### 1. 切换意图存在两个臂，迟到的 `/agent` 结果可覆盖下钻

`/agent` 菜单查询与 `/subtasks` Enter 下钻都通过会话切换落地，但旧实现把
“切到哪个 pick”和“切换成功后采用哪个父会话”分开暂存（pick 与
`arm_after_switch`）。`/agent` 查询是异步的，用户在下钻前发起查询、随后
Enter 下钻时，旧结果可能在下一 tick 到达并改写 pick，导致切到错误会话或
丢失父指针。

### 2. 排队取消仍会被 watcher 的 `error` 覆盖（R2）

上一轮把 `c` 改为发 Server `cancel`，但 `resource_admission` 排队中的执行
取消只写 receipt、不落 execution index；watcher 的 `/index` 仍会 404，
连续 5 轮失败后按 `error` 结算，覆盖用户看到的 `cancelled`。Server 的
cancel 响应本身已携带 `{"id","status"}`，TUI 此前忽略了响应体。

### 3. 无主任务时 Ctrl+Enter 静默（R4）

`append_subtask_result` 在 `main_task_id` 为 `None` 时直接 `let parent =
main_task_id?` 返回，没有 flash；用户按 Ctrl+Enter 看不到任何反馈，也
不知道需要先进入子会话。

## 修复

### 切换意图原子化

- 新增 `crates/tui/src/app_switch.rs`：`PendingSwitch { pick,
  parent_after_switch }` 把切换的两半绑在一起；`stage` 先清空在途
  `/agent` 查询再整体暂存；`accept_polled` 只在暂存槽为空时接受查询结果。
- `app.rs` 消费切换时走 `settle_switch(switched, parent_after_switch)`：
  仅在切换成功时采用父指针，失败即丢弃整份意图；旧的 `arm_after_switch`
  字段删除。
- `/agent` 菜单选择与任务选择器统一走 `stage(plain(pick))`，不再各自拼装
  暂存状态。

### 取消终态本地回填

- `remote/subtasks.rs::cancel` 返回 `CancelOutcome { status:
  Option<ExecutionStatus> }`，解析响应体 `status`；缺失或未知状态为
  `None`，此时仍交给 watcher 继续观察。
- `subtask_detail.rs` 的取消槽记录目标 id；`poll_cancel` 返回 `(id,
  Result<CancelOutcome>)`，即使随后切换了选择也能归属到正确子任务；
  `apply_status` 只改状态，保留摘要与 token。
- `subtask_registry.rs::settle_status_only` 只改状态并保留摘要/token；
  `cancelled` 清空摘要（避免残留传输错误），token 折损保留。
- `subtask_events.rs` 增加本地结算集合（`settled`）与 `settle_local`；
  watcher 通过 `spawn_with_settled` 拿到同一集合，轮询前后都跳过已结算
  目标；事件 drain 忽略已结算目标的迟到更新。
- `app_subtask.rs::tick` 先收取消响应、再收 watcher 回报：终态立即
  `apply_status` + `settle_local` + flash `subtask {id} {status}`；非终态
  只 flash “已请求取消 subtask”，继续观察。

### 无主任务 Ctrl+Enter 提示

- `append_and_pick` 入口校验 `main_task_id`：为空时 flash
  “当前会话没有主任务，无法追加”并直接返回，不暂存任何切换；全屏视图
  仍按原行为关闭。

## 变更清单

- `crates/tui/src/app_switch.rs`（新增）— 切换意图的原子暂存/接受/消费。
- `crates/tui/src/app.rs` — poll/stage/consume 接线，`settle_switch` 采用
  父指针的时机收敛到成功分支。
- `crates/tui/src/app_popups.rs`、`crates/tui/src/app_subtask.rs` — 任务
  选择器改用 `stage(plain(...))`；追加前校验父会话；tick 的取消/watcher
  结算顺序。
- `crates/tui/src/remote/subtasks.rs` — `CancelOutcome` 与 cancel 响应解析。
- `crates/tui/src/subtask_detail.rs`、`subtask_detail_tests.rs` — 取消槽
  携带目标 id、`poll_cancel`/`apply_status`。
- `crates/tui/src/subtask_registry.rs` — `settle_status_only`。
- `crates/tui/src/subtask_events.rs` — `settled` 集合、`settle_local`、
  `maybe_spawn_watcher` 传入集合。
- `crates/tui/src/subtask_watcher.rs` — `spawn_with_settled`/
  `run_with_settled` 与轮询前后跳过已结算目标。
- `crates/tui/src/remote/tests/support.rs`、`remote/tests/subtasks.rs`、
  `remote/mod.rs`、`remote/tests/mod.rs` — 脚本化 cancel 响应与观察器
  测试夹具的可见性。
- `agents/frontend/tui/index.md` — 同步切换意图、取消结算、watcher 跳过与
  无主任务提示的模块口径。
- 本 changelog。

## 不变量

- 一份切换意图（pick + 父指针）只能整体暂存、整体消费；晚到的 `/agent`
  结果不能覆盖已暂存的显式切换，失败的切换不能采用父指针。
- 取消响应携带的终态优先于 watcher 的迟到 `error`/404 回报；未知响应体
  （无 `status`）不改变原观察路径。
- 本地结算只改状态，不重复折算 token；`cancelled` 不保留旧摘要。
- 无主任务时追加动作不产生切换、不清空在途 `/agent` 查询。

## 功能 → 测试名 → 文件

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 切换意图原子暂存 | `staging_descend_cancels_inflight_selection` | `crates/tui/src/app_switch.rs` |
| 迟到查询不覆盖下钻 | `late_poll_cannot_replace_pending_descend` | `crates/tui/src/app_switch.rs` |
| 空槽接受查询结果 | `poll_fills_empty_pending_as_plain` | `crates/tui/src/app_switch.rs` |
| 普通切换不带父指针 | `plain_switch_has_no_parent` | `crates/tui/src/app_switch.rs` |
| 下钻丢弃在途查询 | `enter_descend_clears_pending_agent_selection` | `crates/tui/src/app_subtask.rs` |
| 无主任务 flash 且不暂存 | `ctrl_enter_without_parent_flashes_and_does_not_stage_switch` | `crates/tui/src/app_subtask.rs` |
| 迟到 error 不覆盖本地取消 | `late_watcher_error_does_not_overwrite_local_cancel` | `crates/tui/src/app_subtask.rs` |
| cancel 响应解析 | `cancel_returns_cancelled_status` | `crates/tui/src/remote/tests/subtasks.rs` |
| 非标准响应回退观察 | `cancel_without_status_is_not_terminal` | `crates/tui/src/remote/tests/subtasks.rs` |
| watcher 跳过已结算目标 | `watcher_skips_locally_settled_target` | `crates/tui/src/subtask_watcher.rs` |
| 本地结算保留摘要/token | `settle_status_only_preserves_summary_and_tokens` | `crates/tui/src/subtask_registry.rs` |
| 取消完成带回目标 id | `poll_cancel_returns_target_and_outcome` | `crates/tui/src/subtask_detail_tests.rs` |
| 本地结算写回注册表 | `settle_local_updates_registry_and_counters` | `crates/tui/src/subtask_events.rs` |
| 成功切换才采用父指针 | `switch_landing_adopts_staged_parent` | `crates/tui/src/subtask_events.rs` |
| 失败切换丢弃暂存臂 | `switch_error_drops_staged_arm_and_keeps_current_descent` | `crates/tui/src/subtask_events.rs` |

## 实测

定向测试（实际执行，均 0 failed）：

```text
cargo test -p opencoder-tui --lib app_switch                    # 4 passed
cargo test -p opencoder-tui --lib app_subtask                   # 6 passed
cargo test -p opencoder-tui --lib subtask_events                # 8 passed
cargo test -p opencoder-tui --lib subtask_registry              # 9 passed
cargo test -p opencoder-tui --lib subtask_detail::tests         # 9 passed
cargo test -p opencoder-tui --lib subtask_watcher               # 3 passed
cargo test -p opencoder-tui --lib remote::tests::subtasks       # 9 passed
cargo test -p opencoder-tui --lib app_loop_actions::subtask_append_tests # 4 passed
cargo test -p opencoder-tui --test session_switch_restore       # 3 passed
```

合计 **55 passed / 0 failed**。

- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` →
  exit 0，零警告。
- 格式化：本次触及文件均通过 `rustfmt --edition 2021 --check`；不声称
  仓库全局 fmt 通过（HEAD 上存在与本次无关的未格式化文件，例如
  `crates/tui/tests/agent_mention_flow.rs`）。
- 行数上限：`crates/tui/src/app.rs` = 799 行（上限 800）；
  `app_switch.rs` = 114；`app_subtask.rs` = 614；其余触及文件均 < 800。
- **未运行全量测试**：仅运行上述定向用例，未运行 `cargo test --workspace`、
  SPA 全量测试或全站 UI 验收。

## 已知限制（R1–R5 状态）

- **R1（接受）**：注册表 `apply_update` 的 load/save 仍非事务性；并发更新
  可能丢写。只有实际观察到再升级为 store 事务。
- **R2（本次修复）**：cancel 响应携带终态时本地立即结算，watcher 404 不再
  覆盖；响应体无 `status` 时仍回退到 watcher 的失败计数。
- **R3（接受）**：`fetch_tokens` 失败时显示 0；只影响展示与成本折算。
- **R4（本次修复）**：无主任务时 Ctrl+Enter 给出 flash 提示。
- **R5（另项处理）**：watcher 串行轮询，单个慢目标会延后同轮健康目标的
  观察；本次不改，客户端 30s 超时兜底。

## 回滚

- 依次 revert 本次三个提交（切换意图 `8f921d8f4`、取消回填 `0cb5fa42f`
  与本 changelog 所在提交）；无 schema、Server、配置或依赖迁移。
- 回滚后重新出现旧行为：迟到的 `/agent` 结果可覆盖下钻、排队取消可能被
  watcher 的 `error` 覆盖、无主任务 Ctrl+Enter 静默；历史注册表中的
  `cancelled` 条目仍按普通字符串展示。
- 回滚时删除本 changelog。
