Commit: 37ee51ce6274e0ade8eb9fc8a34da1a1b7e7aa57

# Brain 当前 Error 回执持久化恢复

Store 在 generation CAS 和既有身份校验下，仅允许当前 activation 的 Error 在 paused/blocked 保持原阶段、无取消请求且来源序号递增时恢复为 Done。成功、取消与旧 activation 不可覆盖；提交本身不恢复调度。Core 已有共享工作区恢复判断，Worker 仍通过原生 `layered_terminal` 入口提交。此为当前 main 工作区补丁，尚未发布。

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 当前 Error 在 paused/blocked 恢复，持久化后保持原阶段 | `current_error_recovery_commits_only_while_stopped` | [brain_layered_v4.rs](../../../crates/store/tests/brain_layered_v4.rs) |
| 拒绝旧 activation、取消、旧序号、身份及 generation 变化 | `current_error_recovery_rejects_invalid_state_sequence_or_identity` | [brain_layered_v4.rs](../../../crates/store/tests/brain_layered_v4.rs) |
| Worker 通知经 Core 与 Store 提交并读回，不自动 resume | `recovered_terminal_notice_commits_through_worker_without_resuming` | [tests.rs](../../../crates/worker/src/brain/workspace/tests.rs) |
| Core 共享工作区恢复规则 | `stopped_shared_run_recovers_same_execution_without_spending_a_round` | [recovery.rs](../../../crates/brain/tests/milestone/recovery.rs) |
