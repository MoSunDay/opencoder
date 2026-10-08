# 发布契约整合与容量恢复

- 沿用 libsql schema 33 和 data format 4；项目关联只存执行引用，保留原生结果、能力派发和 Operator 复用接口。旧 SQL 后端和结论缓存不再引入。
- 长结果按所属节点的源版本分块读取；读取失败或版本变化显式报错，展示摘要按 UTF-8 字节限制为 64 KiB。
- Runtime 启动先完成容器与挂载清理、持久化执行恢复，再校验原服务的内核进程及全部票据身份。无法证明清理完成时保留容量；领取后尚未启动的执行先持久化中断。
- 根 Brain 在受理前按冻结模型窗口检查完整证据容量；累计人工输入和恢复不能绕过预算。有关执行节点先升级，再启用严格 Server。
- 发布包记录所需 Runtime 能力。候选 Worker 以完整冻结请求验证容量后才执行发布探针、激活和切换入口；回滚旧包按其原有能力要求验证。

| 行为 | 测试 |
| --- | --- |
| schema 33、长结果与关联不写缓存 | `long_native_conclusion_reads_all_chunks_without_project_cache_writes` |
| 分块失败与重新读取 | `failed_chunk_read_is_visible_and_a_fresh_read_can_retry` |
| DAG 全部步骤 | `dag_native_details_keep_all_sixty_four_steps` |
| 容量恢复不提前释放 | `opening_capacity_never_releases_a_crashed_runtimes_live_slot`、`unverified_restart_retains_capacity_instead_of_trusting_an_empty_tracker` |
| 内核进程归属 | `real_child_is_visible_to_kernel_scan_without_process_tracker_registration` |
| 旧 Runtime 结果与升级回滚 | `released_runtime_results_survive_host_upgrade_sleep_wake_and_rollback` |
| Brain 计划与人工输入超限 | `valid_wide_plan_is_rejected_before_any_child_can_run`、`rejected_human_input_does_not_append_events_or_advance_generation` |
| 发布先确认 Worker 容量支持 | `test_capacity_is_checked_with_frozen_request_before_any_admission`、`test_missing_feature_or_rejected_capacity_prevents_warming_execution` |
| 旧包回滚 | `test_rollback_to_release_without_capacity_requirement_keeps_native_probe` |
