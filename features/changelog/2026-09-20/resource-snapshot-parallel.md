# Agent 资源快照的有界并行复制

测试 DAG 派发在节点显示 ready 时仍返回 504，随后同 ID 才迟到受理。节点在受理锁内同步冻结整个 Agent 资源池；真实样本复制 1,448 个文件的写入跨度达 172.6 秒，超过控制面的 60 秒 RPC 等待窗口。

资源复制按独立资源目录划分，最多使用 16 个线程，并复用目录枚举得到的文件类型，避免 NFS 上重复查询元数据。源目录和符号链接校验、逐文件及目录 fsync、卡片引用校验、staging 原子重命名均保留。所有线程结束后才清理失败的 staging；一个目录失败时不发布快照。既有执行继续使用原冻结目录，重试不读取新资源版本。

快照实现拆为 `resources.rs` 的准入/挂载检查、`resources/snapshot.rs` 的发布和 `resources/copy.rs` 的有界复制。新增 `resource_snapshot` example，可对显式源和全新私有目录测量，不创建平台任务、不修改源资源。

实测只读 NFS 资源池：原串行 106.71 秒，最终并行加目录类型复用 31.07 秒；两份 1,448 文件的路径和 SHA256 全部一致。基准及原始记录位于工作区 `artifacts/test-agent/2026-09-20/e2e-closure/`。该测量不是线上已发布证明；真实派发和全量 gate 尚在验证。

后续隔离 Server/Node 并发实测中，冻结在负载下达到 64/84 秒。清单采集等待受理锁，重连时的 admission 又在 WebSocket 读取循环内等待同一把锁，导致心跳停止并被 Server 判为离线。节点现在独立发送 WebSocket Ping，Server 仅更新已认证连接的存活时间；不伪造新的负载或清单。admission 由连接拥有的有界 FIFO worker 执行，断连时取消未完成的控制请求，退役前等待在途 admission。其他 Create 生命周期及持久受理合同保持原语义。

再次并发实测发现同步资源预检仍会阻塞异步执行线程，导致独立心跳也延迟约 31 秒。新任务预检改为 `spawn_blocking`，配置及隐式资源池在调用线程先解析后传入，保留当前作用域；受理锁仍覆盖完整预检与持久入队，未通过预检不创建执行。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 版本固定及同执行重试 | `parallel_snapshot_freezes_all_cards_current_versions_and_is_retry_stable` | `crates/worker/src/resources/tests.rs` |
| 并发失败不发布、收尾后可重试 | `failed_parallel_copy_publishes_nothing_and_retry_rebuilds_every_entry` | 同上 |
| 缺引用拒绝整个快照 | `missing_agent_reference_never_publishes_a_partial_snapshot` | 同上 |
| 资源内符号链接拒绝 | `symlink_in_any_parallel_resource_rejects_the_entire_snapshot` | 同上 |
| 错误保留具体源路径 | `copy_version_error_names_source_path` | 同上 |
| 清单和受理都阻塞时仍有心跳；退役不丢在途受理；断连取消等待 | `connection_heartbeats_while_admission_and_inventory_are_blocked` | `crates/node/src/fleet/client/tests/heartbeat.rs` |
| 真实 Server/Node 超过离线窗口仍在线，负载与清单不被伪造 | `transport_ping_keeps_node_live_without_fabricating_load_or_inventory` | `crates/control/tests/node_keepalive/main.rs` |
| 慢元数据读取不阻塞单线程 executor，调用方资源池不丢失，错误不受理 | `blocked_resource_read_does_not_starve_node_executor_or_lose_scoped_pool` | `crates/worker/tests/resource_admission/main.rs` |

- 模块测试：5 passed / 0 failed。
- 节点传输模块 7 项通过；真实 Server/Node 存活测试通过。新增退役边界及最终源码仍需完整 gate。
- 前轮 Clippy 零警告，workspace build 通过。全量测试曾在两个 runc Agent E2E 达到 180 秒终态等待上限；降低测试并发后完整 DAG E2E 17 项通过。后续工作区回归发现 Brain 离线重试失败，该路径已由其他变更修复，完整最终 gate 待完成，不据此宣称可上线。
