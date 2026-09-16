Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# 大脑固定图 v2

统一采用 `input → 实例 → output → 路由 → 下一实例 input`。动态生成只负责产生完整计划；运行固定版本，能力来自注册目录，定义和资源快照在发布时固定。手写和生成计划使用同一校验器与纯函数图内核。

局部路由仅接收连线上的输出、语义与相邻输入描述。多输出、并行选择、未选择分支、按因果作用域汇合和回流均保留不可变执行轮次。下游输入传递实际内容及对应轮次的产物引用。完成与验证结论分别记录依据；非法选择、缺少输出、未知交付结论和访问上限明确阻塞。

工作台展示四类概念，提供具名 Markdown 文档提交、输出端口和路由映射编辑、当前分支及路由读集回看。API、CLI、控制面与节点共用 v2；通用执行 API 不能绕过计划注册与快照检查。

旧 ActionFlow、依赖调度、决策树和 Playbook 执行入口退役，历史查询保留。Fleet 协议升级至 10；启动和发布切换前只读检查旧非终态运行，发现后拒绝升级。复用已有版本、事件与回执存储，没有新增数据库表或环境变量。本次不包含生产发布。

## 测试覆盖

| 功能 | 代表测试名 | 文件 |
| --- | --- | --- |
| 手写和动态共用模型 | `generated_and_handwritten_graphs_have_same_validation_and_execution` | `crates/brain/tests/planning.rs` |
| 严格局部路由上下文 | `routing_model_receives_only_local_outputs_semantics_and_candidate_inputs` | `crates/brain/tests/planning.rs` |
| 非法模型输出明确阻塞 | `malformed_route_reply_is_durable_block_not_model_repair_or_agent_fallback` | `crates/brain/tests/planning.rs` |
| 修复复测循环和产物轮次 | `repair_retest_loop_preserves_rounds_and_uses_causal_feedback` | `crates/brain/tests/action_flow.rs` |
| 未知验证、重复回执、暂停取消、访问上限 | `missing_output_and_unsubstantiated_verification_are_not_success`、`replay_pause_cancel_and_visit_limit_are_durable` | `crates/brain/tests/action_flow.rs` |
| 多输出、并行分流、未选择分支与循环 | `parallel_branch_loop_joins_only_final_causal_outputs_and_multiple_ports`、`local_context_and_unselected_branch_do_not_wait_on_global_history` | `crates/brain/tests/execution.rs` |
| 并发子流程不串轮 | `overlapping_parallel_invocations_join_by_cause_not_output_name_or_arrival_order` | `crates/brain/tests/execution.rs` |
| 端口、连线、旧契约与结束路径 | `graph_validator_accepts_loop_and_rejects_legacy_invalid_ports_and_dead_ends` | `crates/brain/tests/ontology.rs` |
| 固定／生成计划和注册能力 | `fixed_plan_executes_through_real_node_channels_and_returns_verified_outputs`、`dynamic_plans_once_publishes_version_and_uses_registered_capabilities` | `crates/worker/tests/brain_ontology.rs` |
| 五类能力的统一回执与输出 | `all_registered_kinds_use_same_receipt_and_out_of_order_notices_cannot_regress`、`managed_team_dag_todo_return_typed_downloadable_outputs` | `crates/brain/tests/execution.rs`、`crates/worker/tests/brain_outputs.rs` |
| 节点重启和恢复重放 | `prepared_action_replays_after_restart_and_duplicate_notice_keeps_watermark` | `crates/worker/tests/brain_recovery.rs` |
| 节点真实循环 | `repair_flow_persists_visits_feedback_and_release_gate_on_node` | `crates/worker/tests/brain_flow.rs` |
| runc 中的真实 CLI 本地激活 | `mounted_cli_finishes_fixed_activation_without_model_credentials` | `crates/worker/src/brain/container.rs` |
| 旧非终态运行阻止升级且历史字节不变 | `legacy_brain_upgrade_guard_preserves_pending_data_and_allows_history`、`test_brain_upgrade_does_not_mutate_old_runs_and_waits_for_idle_roots` | `crates/worker/src/journal/tests.rs`、`scripts/platform/rolling_tests/test_manifest.py` |
| 能力快照与禁止绕过计划入口 | `v2_versions_pin_registered_definitions_and_reject_unregistered_targets`、`generic_execution_cannot_bypass_registered_brain_plan_admission` | `crates/control/tests/e2e/brain_dispatch_extra.rs` |
| 历史查询与旧入口报错 | `legacy_planners_reject_writes_and_preserve_historical_queries` | `crates/web/tests/web_brain_plans.rs` |
| 并发提交生成一次、不重复启动 | `concurrent_v2_runs_generate_once_and_replay_without_new_execution` | `crates/worker/tests/platform/brain_graph.rs` |
| 离线运行不换节点 | `an_unconfirmed_v2_run_never_moves_to_another_node_or_plans_offline` | `crates/worker/tests/platform/brain_graph.rs` |
| 编辑器和提交失败保留输入 | `blocks saving invalid JSON instead of submitting a stale valid binding`、`keeps a durable run identity across an uncertain submission and exposes errors` | `crates/web/spa/src/brain/workbench/tests/` |
| Chromium 编辑、发布、具名文档、真实节点执行及路由回看 | `edit_publish_and_run_graph_in_browser` | `crates/worker/tests/brain_browser.rs`、`scripts/acceptance/brain/runtime.js` |

浏览器用例运行真实服务与节点，以确定性模型回答使结果可复现；没有替换浏览器 API 请求。显式执行：`cargo test -p opencoder-worker --test brain_browser -- --ignored --nocapture`。截图和运行快照输出到用例回执中的临时目录。

2026-09-16 验收：`cargo test --workspace --no-fail-fast --locked` 全部通过（5,222 passed、0 failed、7 ignored），`cargo clippy --workspace --all-targets --locked -- -D warnings` 零警告通过；工作区构建通过。SPA 全量 108 个文件、798 项测试及构建通过，发布前置检查的 Python 20 项测试通过。上述忽略项中的浏览器和 runc 本地激活用例均单独显式执行并通过。浏览器完成编辑、发布、文档提交、真实节点执行及路由回看；本轮截图与快照目录为 `/tmp/opencoder-brain-v2-browser-XdT4C2`。

原始验收回执保存在 `/tmp/brain-workspace-resumed-final.log`、`/tmp/brain-clippy-strict-final.log`、`/tmp/brain-build-resumed-final.log`、`/tmp/brain-spa-acceptance.log`、`/tmp/brain-spa-build-acceptance.log`、`/tmp/brain-browser-acceptance.log`、`/tmp/brain-runc-activation-acceptance.log` 和 `/tmp/brain-release-guard-acceptance.log`。

契约与请求示例见[运行协议](../../../docs/brain-orchestration.md)。
