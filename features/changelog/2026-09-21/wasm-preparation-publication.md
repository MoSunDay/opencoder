# 合并纯 WASM 请求的准备目录发布

执行预检的空资源目录原先在 `durable_create_dir_all` 同步目录和父目录后，又各同步一次；系统调用跟踪确认这些串行同步进入了接收请求的关键路径。

新建纯 WASM 请求将空 Agent 资源目录和 `pending-create.json` 一起放入私有准备目录，在同步目录内容后通过同一次目录重命名发布。原始请求、空资源命名空间及其父目录仍持久化，减少分阶段发布造成的重复同步。需要 Agent 资源的请求保留完整资源池复制、引用校验和隔离；已有准备记录继续按原始输入恢复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 原始输入与空资源命名空间同时发布、重试不改变快照 | `pending_wasm_reservation_already_freezes_its_empty_resource_namespace` | `crates/worker/src/operations/create_retry_tests.rs` |
| 完整资源快照、引用及失败隔离 | `parallel_snapshot_freezes_all_cards_current_versions_and_is_retry_stable`、`failed_parallel_copy_publishes_nothing_and_retry_rebuilds_every_entry` | `crates/worker/src/resources/tests.rs` |
| 已接收请求不受其他冷预检阻塞，重启与重试均忽略遗留预留文件 | `durable_replay_does_not_wait_for_an_unrelated_cold_admission` | `crates/worker/src/operations/create_retry_tests.rs` |
| 发布期间接收和调度连续性 | `exercise`，三个 1 秒时延门槛保持原值 | `scripts/acceptance/smooth_release/main.py`、`metrics.py` |
| 探针超时保留最后一次 Runtime 拒绝原因 | `test_timeout_preserves_the_runtime_rejection` | `scripts/platform/rolling_tests/test_probes.py` |
| 缺少容器镜像时拒绝请求并清理未接收的空资源目录 | `missing_runc_rootfs_is_rejected_before_durable_acceptance` | `crates/worker/src/operations/admission_tests.rs` |

定向准备测试和 5 项资源快照测试通过。最终全量回归、发布包及生产验收结果另记最终候选回执，当前尚未据此宣告上线。

首次混合构建预览的初始探针超时，原回执未记录 RPC 拒绝详情，原因尚未确定；诊断探针返回 200，但完整混合构建预览仍报最大接收延迟 1.1207 秒。以上均不计为通过；探针现保留最后一次 RPC 回复及原始超时异常，便于后续精确诊断。此变更仅减少已确认的冗余同步，不宣称单独解决发布时延。

`c1bd76a5` 全量回归暴露了提前发布空目录的失败清理遗漏：缺少 runc 镜像时，原有测试要求资源目录不存在。修复保留该断言，在 Create 预检拒绝时仅以 `remove_dir` 删除空命名空间并同步父目录；原始预留输入继续保留，已接收请求的重放和 Agent 资源快照不进入此清理路径。非空目录的清理错误会直接报告。

接收日志已通过文件同步、重命名和目录同步落盘后，删除旧预留文件不再追加目录同步。即使崩溃使旧预留文件重新出现，节点恢复与同 ID 请求重放仍以已接收日志为准；该崩溃状态已纳入原有重放测试。此调整不改变接收日志、资源内容或队列容量记录的持久化要求。
