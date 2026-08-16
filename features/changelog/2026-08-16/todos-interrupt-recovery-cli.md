Commit: (working-tree, post-33b5ba2)

# todos 中断/恢复硬化 + CLI run/resume --json / list --limit

## 用户可见变更
- **`todos run|resume --json`**：stdout 只输出最终 workflow 状态文档（单行 compact JSON，默认 pretty），`workflow_id=` 前缀与事件流全部走 stderr——stdout 保持纯 JSON 可机器消费；事件流带 seq 游标，**resume 只回放本次调用产生的事件**。
- **退出码契约**：Completed → 0；本地 Ctrl-C 挂起（`Suspended` + `terminal_reason=local interrupt requested`）→ 130（可恢复）；其它终态（含 doom-loop 守卫/验收卡死等其它 Suspended）→ 1。
- **`todos list --limit N`**（默认 100）：列表条数可控。
- **`todos events <id>`**：未知 workflow 报错（不再静默空输出）。
- **子 session 防自驱动外泄**：todos 执行时强制 `autopilot.mode = Off`（替换旧 `enabled=false`）。

## 核心语义
- **`execution_failed` 转换**（transitions.rs 新增，431 行变更）：Running/CandidateReady/Accepting 失败三态决策——interrupted → `Interrupted`（即使 attempts 耗尽）；`attempt >= max_attempts` → `Failed`；否则 → `NeedsRevision`；同时从 active_todo_ids 摘除。
- **中断回滚全面化**：`reconcile_interrupted` 不再只看 active 列表，直接扫全部 Running/CandidateReady/Accepting → `Interrupted` + 清 candidate + 清 active 列表；删除 `Dispatching` 中间态与 `started` 转换。
- **瞬时 store 错误容忍**：`poll_interrupt` 对 Err warn 后继续轮询（不再误 cancel）；debug dump 失败 warn 降级不阻断运行；`runnable`/`assignments`/`item_records`/`execution`/`parent` 全部改 `with_context` 显式错误（不再裸索引 panic）。
- **batch 执行重构**：`apply_result` 拆分单 todo 错误隔离 + fatal 聚合；`todo_execution_failed` / `todo_failed` 事件落库。
- **todos crate 引入 tracing**：dispatch/accept/interrupt 关键路径结构化日志（Cargo.toml + Cargo.lock 增 tracing 依赖）。
- **`MockChatClient::push_hang`**（llm/mock.rs）：外部 `Notify` 挂起在途 LLM 调用、释放后关流，供确定性中断测试。

## 测试清单（+18 新）
| 测试 | 文件 |
|------|------|
| reconcile 回滚 CandidateReady/Accepting/其余不动（3） | `todos transitions.rs::tests::{reconcile_rolls_back_candidate_ready_todo, reconcile_rolls_back_accepting_todo, reconcile_leaves_other_statuses_untouched}` |
| execution_failed 三态：需修订/耗尽 Failed/interrupted 优先/拒绝 Passed（4） | `todos transitions.rs::tests::{execution_failed_with_attempts_remaining_requests_revision, execution_failed_with_exhausted_attempts_marks_failed, execution_failed_interrupted_marks_interrupted_even_when_exhausted, execution_failed_rejects_passed_todo}` |
| revise 从 accepting 恢复并钉 context_mode / 拒绝 pending / 耗尽 Failed（3） | `todos transitions.rs::tests::{revise_from_accepting_marks_needs_revision_and_pins_context_mode, revise_rejects_pending_todo, revise_with_exhausted_attempts_marks_failed}` |
| rewind 失效后代+重置 milestone / 拒绝非 milestone（2） | `todos transitions.rs::tests::{rewind_invalidates_descendants_and_resets_milestone, rewind_rejects_non_milestone}` |
| 外部 store 级 interrupt 取消在途 todo 且可恢复 | `todos tests/interrupt.rs::external_interrupt_cancels_inflight_todo_and_is_resumable` |
| 本地 cancel 中途标记 Interrupted 并干净停止 | `todos tests/interrupt.rs::local_cancel_mid_todo_marks_item_interrupted_and_stops_cleanly` |
| generation 冲突停止运行 | `todos tests/interrupt.rs::generation_conflict_stops_the_run` |
| 终态 workflow 拒绝再次 interrupt | `todos tests/interrupt.rs::interrupt_rejects_terminal_workflow` |
| 验收期崩溃（kill -9 模拟）resume 自愈 | `todos tests/recovery.rs::acceptance_crash_then_resume_self_heals` |
| parent 拒绝决策使 workflow Failed | `todos tests/recovery.rs::parent_fail_decision_fails_workflow` |
| parent 挂起决策 park workflow | `todos tests/recovery.rs::parent_suspend_decision_parks_workflow` |
| persistence list 返回摘要且尊重 limit | `todos tests/recovery.rs::persistence_list_returns_summaries_and_honors_limit` |
| CLI run/resume --json 解析（含非全局 flag 守卫）+ list --limit 默认 100 | `cli tests/todos_cli_parse.rs`（+4） |
| CLI run/resume 分发（2 新） | `cli tests/todos_cli_dispatch.rs` |
| e2e：run→resume→observe（stdout 纯 JSON / stderr workflow_id= / events 回放） | `scripts/e2e/cli_scenarios.py` E19 |

## 回归 gate
免测提交（用户确认已测，授权 push）：`cargo test --workspace` 全绿、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo build --workspace` 编译干净。行数：transitions.rs 651 / todos_cmd.rs 385 / batch.rs 292 / mock.rs 161 ≤800（迭代）；interrupt.rs 306 / recovery.rs 241 / todos_cli_dispatch.rs 232 ≤400（新增）。
