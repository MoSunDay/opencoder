# Brain run 级暂停/恢复与 resume 附带人工输入

一句话：分层 Brain run（schema 7）支持 run 级 `pause` / `resume` /
`cancel` 控制命令，`resume` 可附带一条人工输入（trim 后 1..=4096 字节，
与 `human_input` 同一累计准入）；run 级暂停与全局事件调度门
（`SchedulerGate`）互不影响，信号镜像沿用 rdb 迁移的 additive mirror
（见
[rdb-store-and-event-framework.md](rdb-store-and-event-framework.md)）。

## 行为口径

- **与全局调度门分离**：`SchedulerGate` 只拦截平台事件分发
  （`dispatch_gated`）；run 级 pause/resume 不读写 `gate/scheduler`，
  事件日志追加与监听照常工作。
- **pause 只停未来调度**：置 `LayeredPhase::Paused`，不改任何 operation
  —— 已启动子任务不被取消、不置 `cancel_requested`，也不发布新的
  `layered_wake`；根 journal 冻结为 `Paused`，结果/错误语义不变。
- **resume 只接受 Paused/Blocked**：running/waiting 或终态 run 一律拒绝；
  barrier 已满足、已有 pending guidance 或本次附带输入时恢复为 `Ready`，
  否则回 `Waiting`；根 journal 回 `Idle`，由常规 outbox 流程发出携带更大
  generation 的 `layered_wake`。仅 resume 可附带一条输入：`text` trim 后
  必须 1..=4096 字节，空串/超长/非字符串返回 400；pause/cancel 携带文本
  直接拒绝。
- **准入先行、零污染**：`budget::human` 把既有 `human_input` 事件与附带
  文本累加进冻结的 guidance 容量，超限返回 413。400/413 都发生在任何
  变更产生之前：零事件、零 generation 递增、零 journal 变化。
- **成功路径顺序**：置 `pending_guidance` 并追加 `human_input` 事件
  （trim 后文本，先于 resume 标记，供 wake 按序重放）→ `run_resumed`
  事件 → 提交即一次 generation 递增并触发新 wake；此前 decision 的拒绝
  原因保留给下一次 decision。
- **store additive mirror**：`commit_brain_layered` 把 `run_paused` /
  `run_resumed` 映射为 `brain.run_paused` / `brain.run_resumed` 平台事件
  （携带 layer/round/activation 坐标），与投影、outbox 同一原子批；不
  触碰 dispatch gate，`operation_terminal` 缺 `execution_id` 仍整批拒绝。

## 调用链与变更清单

- 控制 REST：`POST /api/brain/runs/{id}/commands`
  （`crates/control/src/api/brain_runs/v4/api.rs`，action 白名单 +
  `brain-control` 请求锁，经 `runs::call` 原样转发）；通用
  `POST /api/executions/{id}/commands`
  （`crates/control/src/api/executions/mod.rs`）同样可达。node RPC 侧
  `NodeOperation::Brain` 直达 `crates/worker/src/operations/mod.rs`，
  `NodeOperation::Command` 经 `crates/worker/src/operations/command.rs`
  按 `ExecutionKind::Brain` 分流。
- `crates/worker/src/brain/api.rs` → `crates/worker/src/brain/v4/api.rs`：
  schema 7 分发、文本校验与 `budget::human` 预检（400/413）、提交后经
  `BrainControl::Pause/Resume` 镜像根 journal
  （`crates/worker/src/journal.rs` / `lifecycle.rs`）。
- 预算口径：`crates/worker/src/brain/v4/budget.rs` +
  `crates/brain/src/layered/budget.rs`；纯命令逻辑：
  `crates/brain/src/layered/terminal.rs`（`APPENDED_INPUT_MAX_BYTES =
  4096`）；store 镜像：
  `crates/store/src/rdb_store/execution/brain_layered.rs`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| pause 不取消子任务、不 wake；resume 附带输入落序并触发新 generation wake | `pause_parks_the_root_journal_and_appended_resume_wakes_a_fresh_decision` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 非法附带文本 400 且 run 零变更 | `resume_append_input_rejects_invalid_text_without_mutating_the_run` | 同上 |
| 累计预算超限 413：零事件/零 generation/零 journal | `resume_append_input_over_the_cumulative_budget_returns_413_unchanged` | 同上 |
| Blocked run 经预算命令 + resume 继续下一轮 | `exhausted_budget_blocks_dispatch_until_the_budget_command_and_resume` | 同上 |
| 纯函数：附带输入先于 resume 标记、拒绝原因保留 | `resume_may_append_one_human_input_before_the_resume_marker` | `crates/brain/tests/milestone.rs` |
| 纯函数：仅停机 run 可 resume、仅 resume 可带输入、1..4096 边界 | `resume_append_is_rejected_outside_a_stopped_run` | 同上 |
| 纯函数：暂停保留 pending guidance 供 resume | `paused_run_keeps_pending_human_guidance_for_resume` | 同上 |
| 纯函数：pause 保留 barrier，cancel 接受竞争成功回执 | `pause_retains_the_barrier_and_cancellation_accepts_a_racing_success_receipt` | `crates/brain/tests/milestone/validation.rs` |
| 暂停/恢复镜像 `brain.run_paused` / `brain.run_resumed` 且不动 gate | `brain_control_signals_mirror_into_platform_log` | `crates/store/tests/brain_layered_v4.rs` |
| 全局 gate 暂停时层屏障仍镜像 | `layer_barrier_is_mirrored_to_platform_log_while_paused` | 同上 |
| terminal 缺 execution_id 整批拒绝、outbox 完整 | `operation_terminal_requires_an_execution_identity_for_platform_mirror` | 同上 |
| 控制面：命令白名单 + resume 附带输入原样转发节点 | `layered_commands_forward_only_the_three_lifecycle_actions` | `crates/control/tests/e2e/layered_api/commands.rs` |
| 控制面：human_input 只记 Brain 事件、trim 后转发 | `human_input_is_recorded_only_as_a_brain_event` | 同上 |

- 定向回归（只覆盖本链路，非全量）：

  ```
  cargo test -p opencoder-worker --test brain_scheduler_v4 --quiet
  cargo test -p opencoder-brain --test milestone --quiet
  cargo test -p opencoder-store --test brain_layered_v4 --quiet
  cargo test -p opencoder-control --test e2e layered_api::commands --quiet
  ```
