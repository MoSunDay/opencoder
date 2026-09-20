# Agent 资源快照的有界并行复制

测试 DAG 派发在节点显示 ready 时仍返回 504，随后同 ID 才迟到受理。节点在受理锁内同步冻结整个 Agent 资源池；真实样本复制 1,448 个文件的写入跨度达 172.6 秒，超过控制面的 60 秒 RPC 等待窗口。

资源复制按独立资源目录划分，最多使用 16 个线程，并复用目录枚举得到的文件类型，避免 NFS 上重复查询元数据。源目录和符号链接校验、逐文件及目录 fsync、卡片引用校验、staging 原子重命名均保留。所有线程结束后才清理失败的 staging；一个目录失败时不发布快照。既有执行继续使用原冻结目录，重试不读取新资源版本。

快照实现拆为 `resources.rs` 的准入/挂载检查、`resources/snapshot.rs` 的发布和 `resources/copy.rs` 的有界复制。新增 `resource_snapshot` example，可对显式源和全新私有目录测量，不创建平台任务、不修改源资源。

实测只读 NFS 资源池：原串行 106.71 秒，最终并行加目录类型复用 31.07 秒；两份 1,448 文件的路径和 SHA256 全部一致。基准及原始记录位于工作区 `artifacts/test-agent/2026-09-20/e2e-closure/`。该测量不是线上已发布证明；真实派发和全量 gate 尚在验证。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 版本固定及同执行重试 | `parallel_snapshot_freezes_all_cards_current_versions_and_is_retry_stable` | `crates/worker/src/resources/tests.rs` |
| 并发失败不发布、收尾后可重试 | `failed_parallel_copy_publishes_nothing_and_retry_rebuilds_every_entry` | 同上 |
| 缺引用拒绝整个快照 | `missing_agent_reference_never_publishes_a_partial_snapshot` | 同上 |
| 资源内符号链接拒绝 | `symlink_in_any_parallel_resource_rejects_the_entire_snapshot` | 同上 |
| 错误保留具体源路径 | `copy_version_error_names_source_path` | 同上 |

- 模块测试：5 passed / 0 failed。
- Clippy 零警告，workspace build 通过。全量测试曾在两个 runc Agent E2E 达到 180 秒终态等待上限；其中单项真容器复验通过。限制测试并发后重新进行完整 gate，最终结果待完成，不据此宣称可上线。
