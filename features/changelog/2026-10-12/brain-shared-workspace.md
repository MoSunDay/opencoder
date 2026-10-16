Commit: 445c462eabfe2627b971cb7c904aa07f58a603a4

# Brain 任务共享目录与相邻返工

- 用户要求三 Operator 共用一次 Brain 的工作区；按 activation 留存证据，HOME 独立。
- 节点准入、实际 operation 归属和工作目录恢复使用冻结信息；读回核验声明文件哈希。
- `adjacent_only` 拒绝跨层回退；已有轮次、屏障及失效机制继续生效。

功能 → 定向测试：

- 相邻回退与下游结论失效 → milestone::adjacent::adjacent_workflow_rejects_skipped_return_and_invalidates_later_passes。
- 共享文件、activation 身份、拒绝伪造归属和输入漂移 → workspace_admission_shares_files_and_preserves_activation_identity。
- 拒绝 annotation 覆盖或删除共享目录归属 → workspace_admission_shares_files_and_preserves_activation_identity。
- 原执行环境恢复、保留轮次与暂停、拒绝旧回执 → milestone::recovery::stopped_shared_run_recovers_same_execution_without_spending_a_round。
- 路径限制、声明与内容核验 → workspace_paths_reject_parent_traversal_and_symlinks、shared_artifacts_require_declarations_and_reject_changed_bytes。
