# TODO 画布、执行 Review 与指定节点重跑

TODO 模板新建与编辑默认进入依赖画布。父 Agent 固定负责调度验收，每个 TODO 独立执行；运行工作台提供任务筛选、真实状态、候选结果与验收、实际派发上下文、历史尝试和父子会话 Review。

指定节点重跑先持久化请求，停止旧执行后重置目标与下游，并使用冻结的定义重新排队。上游、独立分支结果、当前文件、外部操作结果和过程历史保留。请求按 request_id 幂等，节点重启继续处理；取消优先于尚未排队的重跑。

同时修复表单往返丢失 TODO metadata、节点改名后依赖引用失效、不可作为 Primary 的 Agent 出现在候选项，以及批量派发事件未更新运行状态的问题。Review 使用快照水位、分页与分段读取；离线、读取失败和停止失败明确显示。

发布检查发现现有 Node 超过 500 个会话时，清单错误地用裸 ID 作为游标，导致持续断线。节点清单与本地 ts 注册迁移现在都使用活动时间和 ID 游标；未操作数据库鉴权数据。

原工作区中的 NFS 自动启动与页面标题调整已对照 `ea2052b2` 核验；发布期间上线的 `c0b88d61` 分页修复与本次修复一致，其回归测试及 `2335f81d` 的只读挂载配置一并保留。

历史回看按工作流事件倒序分页，序号跨工作流存在大跨度时也可直接读取更早记录；大事件继续通过分段读取还原。NFS 状态读取等待生命周期操作完成，避免并发时把运行中的导出误报为停止；启动与目录隔离测试使用独立配置，防止读取宿主机只读挂载。Review 遇到缺失任务状态返回明确错误。

平台浏览器回归还复现了 Brain 的调度周期结束后页面停留旧状态，以及重复 outbox 回执误报。页面现在按同一运行的最终状态继续重连；已采纳的计划发布返回幂等回执，派发授权区分不可变请求身份和可变传输错误，已受理请求返回冲突状态而不再次派发。验收脚本移除了失败后另开运行的路径。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 任意节点影响范围、保留历史与独立结果 | `arbitrary_node_rerun_preserves_unrelated_results_and_history` | `crates/todos/tests/review_contract.rs` |
| 依赖与执行中状态边界 | `rerun_refuses_unaccepted_dependencies_and_live_drivers` | 同上 |
| 子任务依赖正文与父调度精简上下文 | `context_has_dependency_evidence_and_parent_does_not_receive_result_bodies` | 同上 |
| Server/Node Review、大字段、重跑幂等 | `workflow_review_rerun_and_large_context_cross_the_fleet_boundary` | `crates/worker/tests/todo_review.rs` |
| 运行中子任务停止后重新派发 | `rerun_interrupts_an_active_child_before_forking` | 同上 |
| 各持久化检查点恢复一次 | `recovery_finishes_each_durable_rerun_checkpoint_exactly_once` | `crates/worker/src/operations/todo/control_tests.rs` |
| 取消优先与受理检查点 | `cancellation_wins_over_a_pending_rerun_intent`、`cancel_before_store_checkpoint_does_not_restart_a_completed_execution` | 同上 |
| 超过 100 个任务分页与过期版本 | `review_pages_all_nodes_and_rejects_a_stale_page_generation` | 同上 |
| 超过 500 个会话清单 | `node_inventory_reads_more_than_500_sessions_with_activity_cursors` | `crates/worker/src/service.rs` |
| 活动时间不同于创建时间的分页 | `list_all_sessions_paginates_past_store_limit` | `crates/local/src/ts/registry.rs` |
| 事件水位、完整分页、UTF-8 分段、表单保真 | `TODO review consistency` | `crates/web/spa/src/todo/review/contracts.test.js` |
| 会话末尾增量读取 | `polls after the last complete message and does not reload the transcript from zero` | `crates/web/spa/src/todo/review/session.dom.test.jsx` |
| 丢失回执后重试及依赖阻断 | `keeps one request identity across a lost response and closes only after durable queuing`、`unaccepted prerequisites prevent the rerun request` | `crates/web/spa/src/todo/review/rerun.dom.test.jsx` |
| 稀疏事件倒序分页与大字段预算 | `reverse_todo_history_skips_global_sequence_gaps_and_respects_byte_budget` | `crates/store/tests/todos_workflow.rs` |
| 历史分页及固定回看尝试 | `reads sparse history backward with one bounded request per page and selects the latest dispatch`、`keeps a selected historical attempt while live generations advance` | `crates/web/spa/src/todo/review/history.dom.test.jsx`、`inspector.dom.test.jsx` |
| NFS 并发生命周期状态 | `named_export_start_reuse_stop`、`server_startup_starts_the_dag_wasm_export` | `crates/web/src/nfs_exports.rs`、`crates/control/src/bootstrap.rs` |
| Brain 激活间事件重连 | `reloads the same brain run after an activation stream closes and stops at its terminal state` | `crates/web/spa/src/brain/workbench/tests/reconnect.dom.test.jsx` |
| Brain 重启、重复回执与请求身份校验 | `prepared_action_replays_after_restart_and_duplicate_notice_keeps_watermark` | `crates/worker/tests/brain_recovery.rs` |
| 浏览器全流程、断线与重启历史 | `scripts/acceptance/todo_workbench/main.js` | 截图与 result.json 保存在脚本输出的独立临时目录 |

## 验证记录

- 前端全量：705 passed，0 failed（`/tmp/todo-spa-final-complete.log`）。
- 浏览器：PASS（`/tmp/opencoder-todo-workbench-eBU1vC/result.json`）。
- 平台浏览器：PASS（`/tmp/todo-platform-isolated.log`；证据目录 `/tmp/opencoder-t12-verify-wf9nHp`），同一 Brain 运行完成，不另开运行替代。
- SPA 构建与 drift：通过。
- Rust 全量和最终发布记录将在门禁完成后补充。
