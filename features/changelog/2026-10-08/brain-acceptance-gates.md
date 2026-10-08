# 大脑上下文容量、崩溃处理与真实验收

大脑准入按冻结模型的实际窗口预留完整计划、能力约定、执行证据、人工输入和 16384 输出 token；超限请求返回 413，不先创建运行，人工输入拒绝也不追加事件或推进代次。节点通过 `brain_context_budget_v1` 声明支持该检查，运行中的完整上下文超限则阻塞，保留证据。

按用户决定，Worker 或决策进程意外崩溃会使根运行整体失败，迟到回执不能复活或继续派发。正常退出和 Server 重启保持连续。离线 `storage settle-brain-crash` 必须验证所属 Runtime 停止、进程清理及目录独占锁；先保存意图与失败状态，再结清该根的精确容量票据，重复执行保持结果，不操作鉴权数据。

已结束根运行的人工输入、暂停、恢复、取消和预算调整返回明确的 409 状态冲突，并保持完整状态不变；此前终态拒绝被通用错误处理转换为 500，容易被调用方误认成可重试服务错误。

Agent 日志写入 stderr，离线存储命令的 stdout 仅输出 JSON。真实 Worker 崩溃验收发现，容量结清成功后 stdout 混入初始化日志，导致回执无法解析；修正输出通道后，`worker-crash` 同时验证实际 JSON、严格停止证明、精确结清及重复调用结果。

实时指导用例等待 Agent、Operator、Team 的真实本地工具均进入等待状态，观察到三者的指导事件并核对该事件的全部投递确认后释放工具，再检查三个实际最终结果中的新标记。替换固定 150 秒休眠，避免目标提前结束；本地等待最多 600 秒，超时仍判失败，只允许操作本批执行的工作区。

独立环境使用 5 个名额：四个等待中的子执行和一次大脑指导。只有 4 个名额时，指导会正常排队至某个子执行结束，无法证明运行中的三类指导。指导用例先核对该条件；满载排队用例仍临时设为 1 个名额，并核对恢复原配置，容量校验不变。

故障窗口允许自身冻结造成的 `node admission is frozen`，仍拒绝其他执行、其他忙碌节点及资源故障。恢复先核对 Runtime 身份并解除冻结，再等待公共节点真正就绪，避免要求冻结节点先就绪而无法解冻。

Server 重启后等待实际监听及所属节点重新连接，再读取根运行。模型故障注入的内部检查同样允许自身冻结，只更改停止 Runtime 后本批根运行的冻结传输地址。派发重试用例采用一轮预算，避免将模型主动返工误认成同一创建意图重试；控制场景的根输入与节点说明统一使用 `a + b`。

离线派发先等待所属 Worker 实际开始本代决策，再停止 Server；只收到运行 ID 不能证明模型已启动。用例失败后先取消该用例自己的根运行并等待子执行结束，再运行下一项；清理不完成时停止后续用例并记为未执行，避免失败任务污染后续故障窗口。

普通子执行仍由统一调度器各自选点。验收先读取子执行的实际节点，再按节点身份、根运行及 operation 身份核对本机托管 Runtime 的日志目录；只在唯一匹配的本批工作区释放等待文件，不再假设子执行位于根 Runtime。发现远端目录不可访问、身份不符或多个目录时明确失败，不改变产品选点规则。

| 功能 | 测试 |
| --- | --- |
| 模型窗口、转义、冻结定义及宽计划容量 | `valid_wide_plan_is_rejected_before_any_child_can_run`、`model_capacity_and_json_escaping_are_included_in_admission`、`nested_receipts_and_large_frozen_definitions_are_reserved` |
| 准入拒绝不创建执行、人工输入拒绝不改状态 | `probe_and_create_reject_a_plan_before_execution_admission`、`rejected_human_input_does_not_append_events_or_advance_generation`、真实 `context-capacity` |
| 正常退出连续、意外崩溃失败、决策 panic 不重试 | `unclean_worker_exit_fails_waiting_root_and_clean_restart_retains_it`、`decision_task_panic_fails_projection_and_journal_without_retry`、`restart_settles_a_failure_committed_before_its_journal_without_another_event` |
| 失败根的五类控制命令返回 409 且不改事件、代次或状态 | `unclean_worker_exit_fails_waiting_root_and_clean_restart_retains_it`；真实 `process-recovery`、`worker-crash`、`decision-crash` |
| 严格停止证明、cgroup 两种布局、精确恢复 | `stopped_proof_checks_processes_in_both_systemd_cgroup_layouts`、`unknown_active_or_owned_runtime_cannot_release_capacity`、真实 `worker-crash`、`decision-crash` |
| 实际工具保持运行、三者均就绪后指导、释放后返回 | `test_actual_tools_wait_for_all_targets_and_controller_release`、真实 `live-steering` |
| 满载前置条件不足时不创建指导任务 | `test_full_four_slot_node_is_rejected_before_creating_guidance_work` |
| 指导已决策但未投递、旧事件确认不释放工具 | `test_old_or_missing_delivery_receipt_cannot_release_current_guidance` |
| 没有指导释放时超时失败 | `test_timeout_fails_without_controller_release` |
| 拒绝其他根、DAG、目录越界与软链接越界 | `test_foreign_roots_dags_and_escaping_workspaces_are_refused` |
| 解冻身份、资源和公共就绪检查 | `test_fault_restore_waits_for_owned_runtime_and_public_node_readiness` |
| 自身冻结不隐藏其他任务或资源错误 | `test_fault_scope_requires_exact_owned_ids_and_idle_other_nodes` |
| 新 PID 不代替监听及所属节点重新连接 | `test_server_restart_waits_for_listener_and_owned_node_connection` |
| 故障只改变本批冻结传输地址，保留凭证、意图及预算 | `test_stopped_owned_fixture_changes_only_its_frozen_transport` |
| 离线前已开始本代决策，不能以 Ready 或旧尝试代替 | `test_offline_fault_waits_for_matching_actual_decision_attempt` |
| 失败用例清理只处理本用例，不影响其他用例或外部根 | `test_failed_case_cleanup_cannot_cancel_other_case_or_foreign_roots` |
| 子执行位于另一节点时使用实际所属工作区 | `test_unpinned_child_uses_exact_owner_instead_of_root_runtime` |
| 拒绝其他根、其他 operation 和多个匹配目录 | `test_foreign_parent_operation_and_ambiguous_owners_are_refused` |
| 只从 Runtime 进程读取数据目录，不读取凭证配置 | `test_process_discovery_does_not_read_config_or_accept_other_binaries` |

测试入口：`python3 -m unittest discover -s scripts/acceptance/brain/scheduling/tests -v`。真实故障用例须在独立托管环境运行；隔离配置须包含实际 embedding 提供方，以便通过能力登记 API 完成约定验收。

Rust 测试入口见 [context_budget.rs](../../../crates/brain/tests/context_budget.rs)、[brain_context_budget.rs](../../../crates/worker/tests/brain_context_budget.rs)、[brain_crash.rs](../../../crates/worker/tests/brain_crash.rs)、[停止证明](../../../crates/worker/src/runtime/crash/proof.rs)。完整回归使用 `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`cargo build --workspace`；线上验收同时核对六个成套二进制的真实编译版本。
