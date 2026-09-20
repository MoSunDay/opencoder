# 合并纯 WASM 请求的准备目录发布

执行预检的空资源目录原先在 `durable_create_dir_all` 同步目录和父目录后，又各同步一次；系统调用跟踪确认这些串行同步进入了接收请求的关键路径。

新建纯 WASM 请求将空 Agent 资源目录和 `pending-create.json` 一起放入私有准备目录，在同步目录内容后通过同一次目录重命名发布。原始请求、空资源命名空间及其父目录仍持久化，减少分阶段发布造成的重复同步。需要 Agent 资源的请求保留完整资源池复制、引用校验和隔离；已有准备记录继续按原始输入恢复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 原始输入与空资源命名空间同时发布、重试不改变快照 | `pending_wasm_reservation_already_freezes_its_empty_resource_namespace` | `crates/worker/src/operations/create_retry_tests.rs` |
| 完整资源快照、引用及失败隔离 | `parallel_snapshot_freezes_all_cards_current_versions_and_is_retry_stable`、`failed_parallel_copy_publishes_nothing_and_retry_rebuilds_every_entry` | `crates/worker/src/resources/tests.rs` |
| 已接收请求不受其他冷预检阻塞 | `durable_replay_does_not_wait_for_an_unrelated_cold_admission` | `crates/worker/src/operations/create_retry_tests.rs` |
| 发布期间接收和调度连续性 | `exercise`，三个 1 秒时延门槛保持原值 | `scripts/acceptance/smooth_release/main.py`、`metrics.py` |
| 探针超时保留最后一次 Runtime 拒绝原因 | `test_timeout_preserves_the_runtime_rejection` | `scripts/platform/rolling_tests/test_probes.py` |

定向准备测试和 5 项资源快照测试通过。最终全量回归、发布包及生产验收结果另记最终候选回执，当前尚未据此宣告上线。

首次混合构建预览的初始探针超时，原回执未记录 RPC 拒绝详情，原因尚未确定；诊断探针返回 200，但完整混合构建预览仍报最大接收延迟 1.1207 秒。以上均不计为通过；探针现保留最后一次 RPC 回复及原始超时异常，便于后续精确诊断。此变更仅减少已确认的冗余同步，不宣称单独解决发布时延。
