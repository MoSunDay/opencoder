Commit: (working-tree, post-b663adb)

# feat(todos,cli): todos 中断/崩溃容错硬化 + CLI 观测面（**BREAKING**：run/resume stdout 改纯 JSON）

## 背景

todos 工作流此前对"进程在半途死掉 / 外部 interrupt / 批内单项失败"三类故障没有闭环：
resume 只把 `Running` 归约、遗留 stale candidate；批执行一个 TODO 报错就丢掉整批结果；
CLI 把 `workflow_id=` 前缀文本和最终状态混在 stdout，脚本无法稳定解析，Ctrl-C 退出码与
普通失败不可区分。本轮把状态机、批执行与 CLI 输出合同一次性收紧，并补齐回归测试。

## 变更

### crates/todos 运行时（状态机 + 批执行 + 容错）

- **`src/transitions.rs`**（651 行，+419/−12）：
  - `reconcile_interrupted`：回滚范围从仅 `Running` 扩为 `Running | CandidateReady | Accepting`
    → `Interrupted`，同时清空 stale `candidate`；`active_todo_ids` 清空、`world_epoch`/`incidents`
    不动（父模型全局记忆不因中断丢失）。
  - 新增 `execution_failed(spec, state, todo_id, reason, interrupted)`：有剩余 attempt →
    `NeedsRevision`；attempt 用尽 → `Failed`；`interrupted` 优先落 `Interrupted`（即使用尽）；
    拒绝对 `Passed` TODO 施加。模块保持纯函数，无 tracing/IO。
  - `revise`：补 `from Accepting` 合法性 + `next_context_mode` 钉扎 + attempt 用尽 → Failed +
    拒绝 `Pending`；`rewind`：失效全部后代、重置 milestone TODO 自身、拒绝非 milestone。
- **`src/batch.rs`**（303 行，+110/−27）：`execute` 逐项应用执行结果——执行错误经
  `execution_failed` 落 NeedsRevision/Failed/Interrupted 并 commit；**兄弟 TODO 结果恒持久化**
  （单项失败不吞掉同批成功项）；只有 fatal（转换/commit 失败）才让整个 run 挂起。
- **`src/runner.rs`**（310 行，+26/−3）：`poll_interrupt` 容忍瞬时 store 错误（`Err` → warn 继续
  轮询；`Ok(None)`/generation 不匹配/已 suspended → cancel）；`Runtime::dump` 改 best-effort
  （投影失败 warn 而非让 run 失败）。
- **`src/domain.rs`**（318 行，+146/−20）：`runnable` 返回 `Result`（状态缺失带上下文报错）；
  `validate_spec` 拒绝空 instructions/非对象 tool 参数/max_attempts=0；非测试代码全部
  `state.todos[&id]` 索引改 `.get()` + context。
- **`src/types.rs`**（−2）：删除从未提交过的死状态 `Dispatching` 与 `started()`。
- **`src/persistence.rs`**（198 行，+11/−4）：新增 `list(store, limit)` 汇总查询。
- **tracing（本轮 WP4 补齐）**：`batch.rs` 6 处 + `runner.rs` 2 处 `tracing::info!`——dispatch
  提交后（todo ids + reason）、单项执行失败（todo_id + interrupted 标志）、accept/revise/fail/
  rewind 决定落库后、`interrupt()`（workflow_id + reason）、`drive()` 持久化 runtime_error 挂起时
  （workflow_id + error）。字段只含 ID/标志/理由，不落 prompt、密钥或全量 state。

### crates/llm（测试原语）

- **`src/mock.rs`**（+88/−10）：`MockChatClient::push_hang(notify)`——下一次 `chat_stream`
  静默挂起直到 `Notify` 触发，用于把"执行中的 TODO"钉在 flight 中再模拟外部 interrupt。

### crates/cli（todos UX，**BREAKING**）

- **`src/todos_cmd.rs`**（385 行，+225/−23）+ **`src/lib.rs`**（+13）：
  - **BREAKING**：`todos run/resume` 的 stdout 现在是**纯最终状态 JSON 文档**（无 `workflow_id=`
    前缀、无尾注），`workflow_id=` 移到 **stderr**；新增 `--json` 紧凑单行模式（默认 pretty）。
  - run/resume 期间 stderr 起进度 tailer（逐条打印 transition 事件行），不触碰 stdout。
  - 退出码合同：completed → 0；本地 Ctrl-C 挂起 → **130**（可 resume）；其他终态 → 1 并携带
    状态名（`todos_terminal_outcome`/`TodosOutcome`/`render_final_state`）。
  - `events <id>` 对不存在的工作流报错（exit 1）而非静默空列表；`interrupt` 错误带 workflow id；
    spec 解析错误上下文带文件路径；`list` 新增 `--limit`（默认 100）。
- **`crates/cli/tests/todos_cli_dispatch.rs`**（新，232 行，9 用例）+ `todos_cli_parse.rs`
  （+74/−2，7 用例）：输出合同、退出码、flag 解析全覆盖。

### e2e

- **`scripts/e2e/cli_scenarios.py`**（+95/−3）：E19 todos 冒烟——run（stdout 纯 JSON + stderr
  workflow_id=）→ events/show/list 观测链 → 终态 resume 幂等返回；todo passed 与落盘 artifact
  为 SOFT 检查。

## 已知边界（诚实记录，不修）

- **外部 interrupt 的提交竞态**：另一进程执行 `todos interrupt` 时，在飞 run 的逐项 commit 会
  撞上 generation 乐观锁，单项落 `NeedsRevision`（而非 `Interrupted`），run 自身持久化
  `runtime_error` 挂起；`resume` 后照常自愈（reconcile 归约 + 重派发）。本地 Ctrl-C 路径按设计
  把在飞项标 `Interrupted`。集成测试 `external_interrupt_cancels_inflight_todo_and_is_resumable`
  断言的是可恢复性而非单项终态。
- dump best-effort 的"投影失败不致命"路径无直接失败注入用例（由 debug 投影刷新用例 +
  编译期签名约束覆盖）。

## 功能 → 测试名 → 文件

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| reconcile 回滚 CandidateReady（清 candidate） | `reconcile_rolls_back_candidate_ready_todo` | `crates/todos/src/transitions.rs` |
| reconcile 回滚 Accepting | `reconcile_rolls_back_accepting_todo` | `crates/todos/src/transitions.rs` |
| reconcile 不动其他状态/world_epoch | `reconcile_leaves_other_statuses_untouched` | `crates/todos/src/transitions.rs` |
| execution_failed 有剩余 attempt → NeedsRevision | `execution_failed_with_attempts_remaining_requests_revision` | `crates/todos/src/transitions.rs` |
| execution_failed attempt 用尽 → Failed | `execution_failed_with_exhausted_attempts_marks_failed` | `crates/todos/src/transitions.rs` |
| execution_failed interrupted 优先 Interrupted | `execution_failed_interrupted_marks_interrupted_even_when_exhausted` | `crates/todos/src/transitions.rs` |
| execution_failed 拒绝 Passed | `execution_failed_rejects_passed_todo` | `crates/todos/src/transitions.rs` |
| revise：Accepting→NeedsRevision + context_mode 钉扎 | `revise_from_accepting_marks_needs_revision_and_pins_context_mode` | `crates/todos/src/transitions.rs` |
| revise 拒绝 Pending | `revise_rejects_pending_todo` | `crates/todos/src/transitions.rs` |
| revise attempt 用尽 → Failed | `revise_with_exhausted_attempts_marks_failed` | `crates/todos/src/transitions.rs` |
| rewind 失效后代并重置 milestone | `rewind_invalidates_descendants_and_resets_milestone` | `crates/todos/src/transitions.rs` |
| rewind 拒绝非 milestone | `rewind_rejects_non_milestone` | `crates/todos/src/transitions.rs` |
| validate 拒绝未知依赖 | `validate_spec_rejects_unknown_dependency` | `crates/todos/src/domain.rs` |
| validate 拒绝自依赖 | `validate_spec_rejects_self_dependency` | `crates/todos/src/domain.rs` |
| validate 拒绝 max_attempts=0 | `validate_spec_rejects_zero_max_attempts` | `crates/todos/src/domain.rs` |
| validate 拒绝空 instructions | `validate_spec_rejects_empty_instructions` | `crates/todos/src/domain.rs` |
| validate 拒绝非对象 tool 参数 | `validate_spec_rejects_non_object_tool_arguments` | `crates/todos/src/domain.rs` |
| validate 菱形 DAG + runnable 顺序 | `validate_spec_accepts_diamond_and_runnable_orders_correctly` | `crates/todos/src/domain.rs` |
| 嵌套 JSON 参数子集 | `nested_json_subset_is_supported` | `crates/todos/src/domain.rs` |
| 工具门禁：缺 required call 拒绝 | `gate_rejects_when_required_call_missing` | `crates/todos/src/execution.rs` |
| 工具门禁：错误 ToolEnd 拒绝 | `gate_rejects_errored_tool_end` | `crates/todos/src/execution.rs` |
| 验收窗口崩溃 → runtime_error 挂起 + resume 自愈（CandidateReady/Accepting 回滚、事件 append-only、resume 复用崩溃 session） | `acceptance_crash_then_resume_self_heals` | `crates/todos/tests/recovery.rs` |
| 父 fail 决定落终态（批结果逐项应用 + 持久化） | `parent_fail_decision_fails_workflow` | `crates/todos/tests/recovery.rs` |
| 父 suspend 决定 park 工作流 | `parent_suspend_decision_parks_workflow` | `crates/todos/tests/recovery.rs` |
| persistence::list 汇总 + limit 截断 | `persistence_list_returns_summaries_and_honors_limit` | `crates/todos/tests/recovery.rs` |
| 外部 interrupt 取消在飞 TODO 且可 resume（含轮询瞬时错误容忍） | `external_interrupt_cancels_inflight_todo_and_is_resumable` | `crates/todos/tests/interrupt.rs` |
| 本地 Ctrl-C：单项标 Interrupted、干净停车 | `local_cancel_mid_todo_marks_item_interrupted_and_stops_cleanly` | `crates/todos/tests/interrupt.rs` |
| generation 冲突（外部写入）停止本 run | `generation_conflict_stops_the_run` | `crates/todos/tests/interrupt.rs` |
| interrupt 拒绝终态工作流 | `interrupt_rejects_terminal_workflow` | `crates/todos/tests/interrupt.rs` |
| 中断挂起后 TODO 可恢复可派发 | `suspended_active_todo_becomes_recoverable_and_runnable` | `crates/todos/tests/runtime.rs` |
| debug 投影刷新（dump 路径行为） | `existing_debug_projection_refreshes_after_external_state_change` | `crates/todos/tests/runtime.rs` |
| 非调试运行不落目录 | `normal_execution_does_not_create_a_debug_projection` | `crates/todos/tests/runtime.rs` |
| 多 TODO 单批并发 | `parent_can_dispatch_multiple_independent_todos_in_one_batch` | `crates/todos/tests/runtime.rs` |
| 单 TODO 走到完成 | `parent_drives_focused_primary_todo_to_completion` | `crates/todos/tests/runtime.rs` |
| 依赖环校验 + runnable 依赖感知 | `dependency_validation_rejects_cycles_and_runnable_is_dependency_aware` | `crates/todos/tests/runtime.rs` |
| Dispatching 死状态删除（编译期：状态枚举无该成员、无引用） | —（编译级断言，无运行时用例） | `crates/todos/src/types.rs` |
| 非测试代码安全索引（编译期：`[]` panic 点清零，`.get()`+context） | —（编译级断言，无运行时用例） | `crates/todos/src/{domain,batch,runner}.rs` |
| CLI events 未知 id 报错 exit 1 | `events_unknown_id_errors_with_exit_context` | `crates/cli/tests/todos_cli_dispatch.rs` |
| CLI show 未知 id 报错 | `show_unknown_id_errors` | `crates/cli/tests/todos_cli_dispatch.rs` |
| CLI list 空目录零输出零退出 | `list_on_empty_workdir_outputs_nothing_and_exits_zero` | `crates/cli/tests/todos_cli_dispatch.rs` |
| CLI validate 解析错误带文件路径 | `validate_reports_file_path_on_bad_spec` | `crates/cli/tests/todos_cli_dispatch.rs` |
| CLI validate 合法 spec 通过 | `validate_accepts_good_spec` | `crates/cli/tests/todos_cli_dispatch.rs` |
| **BREAKING** stdout 纯状态 JSON（compact/pretty 同文档、无 workflow_id= 文本） | `render_final_state_pretty_vs_json` | `crates/cli/tests/todos_cli_dispatch.rs` |
| 退出码映射 0/130/1（非 Ctrl-C 挂起不误报 130） | `terminal_outcome_maps_completed_interrupted_and_ended` | `crates/cli/tests/todos_cli_dispatch.rs` |
| CLI interrupt 错误携带 workflow id | `interrupt_unknown_workflow_errors_with_id` | `crates/cli/tests/todos_cli_dispatch.rs` |
| CLI run 配置不可解析时干净失败 | `run_with_unresolvable_config_fails_cleanly` | `crates/cli/tests/todos_cli_dispatch.rs` |
| 解析：run/resume/debug 作用域 | `parses_todos_run_resume_and_debug_scope` | `crates/cli/tests/todos_cli_parse.rs` |
| 解析：run/resume `--json` flag | `parses_todos_run_and_resume_json_flag` | `crates/cli/tests/todos_cli_parse.rs` |
| 解析：`list --limit` 默认 100 | `parses_todos_list_limit_with_default_100` | `crates/cli/tests/todos_cli_parse.rs` |
| 解析：`--json` 非全局/非 validate flag | `json_flag_is_not_a_global_or_validate_flag` | `crates/cli/tests/todos_cli_parse.rs` |
| 解析：show/events 保留既有 `--json` | `show_and_events_keep_their_pre_existing_json_flag` | `crates/cli/tests/todos_cli_parse.rs` |
| 解析：`--debug` 非全局/非 show flag | `debug_is_not_a_global_or_show_flag` | `crates/cli/tests/todos_cli_parse.rs` |
| 解析：validate 不吃 runtime flags | `parses_todos_validate_without_runtime_flags` | `crates/cli/tests/todos_cli_parse.rs` |
| MockChatClient.push_hang 钉住流再释放 | `push_hang_holds_stream_silent_then_ends_after_release` | `crates/llm/src/mock.rs` |
| e2e E19：run stdout 纯 JSON / stderr workflow_id= / events-show-list 观测链 / 终态 resume 幂等（HARD） | E19 `todos workflow smoke (run -> resume -> observe)` | `scripts/e2e/cli_scenarios.py` |
| e2e E19：todo passed + artifact 落盘（SOFT） | E19 soft checks | `scripts/e2e/cli_scenarios.py` |

## 回归

- `cargo clippy --workspace --all-targets -- -D warnings` → PASS（0 警告，exit 0）
- `cargo test --workspace` → PASS（**2822 passed / 0 failed**，172 个 suite，exit 0；基线
  2729/165 → 净增 93，无删除）
