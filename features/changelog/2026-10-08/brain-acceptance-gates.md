# 大脑上下文容量、崩溃处理与真实验收

大脑准入按冻结模型的实际窗口预留完整计划、能力约定、执行证据、人工输入和 16384 输出 token；超限请求返回 413，不先创建运行，人工输入拒绝也不追加事件或推进代次。节点通过 `brain_context_budget_v1` 声明支持该检查，运行中的完整上下文超限则阻塞，保留证据。

按用户决定，Worker 或决策进程意外崩溃会使根运行整体失败，迟到回执不能复活或继续派发。正常退出和 Server 重启保持连续。离线 `storage settle-brain-crash` 必须验证所属 Runtime 停止、进程清理及目录独占锁；先保存意图与失败状态，再结清该根的精确容量票据，重复执行保持结果，不操作鉴权数据。

实时指导用例等待 Agent、Operator、Team 的真实本地工具均进入等待状态，在观察到三者的指导事件后释放工具，并检查三个实际最终结果中的新标记。替换固定 150 秒休眠，避免模型决策耗时导致目标提前结束；本地等待最多 600 秒，超时仍判失败，只允许操作本批执行的工作区。

独立环境使用 5 个名额：四个等待中的子执行和一次大脑指导。只有 4 个名额时，指导会正常排队至某个子执行结束，无法证明运行中的三类指导。指导用例先核对该条件；满载排队用例仍临时设为 1 个名额，并核对恢复原配置，容量校验不变。

故障窗口允许自身冻结造成的 `node admission is frozen`，仍拒绝其他执行、其他忙碌节点及资源故障。恢复先核对 Runtime 身份并解除冻结，再等待公共节点真正就绪，避免要求冻结节点先就绪而无法解冻。

| 功能 | 测试 |
| --- | --- |
| 模型窗口、转义、冻结定义及宽计划容量 | `valid_wide_plan_is_rejected_before_any_child_can_run`、`model_capacity_and_json_escaping_are_included_in_admission`、`nested_receipts_and_large_frozen_definitions_are_reserved` |
| 准入拒绝不创建执行、人工输入拒绝不改状态 | `probe_and_create_reject_a_plan_before_execution_admission`、`rejected_human_input_does_not_append_events_or_advance_generation`、真实 `context-capacity` |
| 正常退出连续、意外崩溃失败、决策 panic 不重试 | `unclean_worker_exit_fails_waiting_root_and_clean_restart_retains_it`、`decision_task_panic_fails_projection_and_journal_without_retry`、`restart_settles_a_failure_committed_before_its_journal_without_another_event` |
| 严格停止证明、cgroup 两种布局、精确恢复 | `stopped_proof_checks_processes_in_both_systemd_cgroup_layouts`、`unknown_active_or_owned_runtime_cannot_release_capacity`、真实 `worker-crash`、`decision-crash` |
| 实际工具保持运行、三者均就绪后指导、释放后返回 | `test_actual_tools_wait_for_all_targets_and_controller_release`、真实 `live-steering` |
| 满载前置条件不足时不创建指导任务 | `test_full_four_slot_node_is_rejected_before_creating_guidance_work` |
| 没有指导释放时超时失败 | `test_timeout_fails_without_controller_release` |
| 拒绝其他根、DAG、目录越界与软链接越界 | `test_foreign_roots_dags_and_escaping_workspaces_are_refused` |
| 解冻身份、资源和公共就绪检查 | `test_fault_restore_waits_for_owned_runtime_and_public_node_readiness` |
| 自身冻结不隐藏其他任务或资源错误 | `test_fault_scope_requires_exact_owned_ids_and_idle_other_nodes` |

测试入口：`python3 -m unittest discover -s scripts/acceptance/brain/scheduling/tests -v`。真实故障用例须在独立托管环境运行；隔离配置须包含实际 embedding 提供方，以便通过能力登记 API 完成约定验收。

Rust 测试入口见 [context_budget.rs](../../../crates/brain/tests/context_budget.rs)、[brain_context_budget.rs](../../../crates/worker/tests/brain_context_budget.rs)、[brain_crash.rs](../../../crates/worker/tests/brain_crash.rs)、[停止证明](../../../crates/worker/src/runtime/crash/proof.rs)。完整回归使用 `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`cargo build --workspace`；线上验收同时核对六个成套二进制的真实编译版本。
