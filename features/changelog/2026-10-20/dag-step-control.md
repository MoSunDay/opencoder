Commit: a584ef83df8edae415f9c4fe6a7aac995a7b9614

# DAG 单步控制与原会话继续

新增可选 `recovery_policy: operator` 与 `dag_step_control_v1`。ctl 的 `dag steps` 提供查询、暂停、继续、重试和明确失败；命令与回执持久保存，核对尝试次数。未完成实例可单独恢复或整组控制，已完成结果保持。

Agent 恢复原 session、消息和输入消费记录。普通步骤默认失败行为不变；没有步骤期限的定义仍通过省略 `timeout_secs` 表达。

功能与验证对应：

- 目标进程树隔离、收尾等待：`operator_pause_reaps_only_target_and_defers_finalizer_until_resume`。
- 实例局部重试：`operator_dynamic_failure_waits_without_cancelling_other_instances`。
- 原会话及原输入唯一性：`operator_agent_resume_keeps_session_and_does_not_repeat_original_input`。
- 尝试历史、状态重建、取消及整组暂停：`runtime::managed::tests`。
- Worker 持久回执和容量：`dag_step_commands_are_durable_attempt_checked_and_do_not_cancel_the_run`。
- Worker 进程强杀后原 DAG 续跑、已完成步骤不重复及旧 context 归档：`killed_worker_resumes_original_dag_without_replaying_completed_step`。
- ctl 参数：`dag_step_control_preserves_attempt_and_operation_identity`。

这些是源码与真实 runc 定向验收；在线发布验收尚未执行。使用方式见[单步恢复](../../../docs/dag-step-control.md)。
