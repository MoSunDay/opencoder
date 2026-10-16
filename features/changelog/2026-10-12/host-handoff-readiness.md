Commit: 5afffa4f8dfce22263f04134d0cb111f9e29c42f

# Host 交接等待索引同步，存储等待不阻塞心跳

兼容发布期间，全量索引写入在 WebSocket 主循环内等待同步 SQLite，可能令真实在线的 Host 心跳无法被读取。部署脚本激活 Host 后立即切入口，也可能将请求转向尚未同步索引的新连接。

## 当前行为

- 通道按原顺序处理应用帧；独立处理传输心跳与写出。应用帧队列为 128 项，另保留一项待入队帧；队列满时暂停读取，仍可完成 RPC 与交接确认。
- 通道数据库操作顺序等待阻塞线程执行，避免 SQLite 的 busy timeout 占用 Tokio 执行线程。索引提交且通过代次栅栏后才确认交接，确认等待真实 socket 写出。
- 断线取消原连接的处理 future；已开始的事务可完成，但不能继续标记替代连接就绪或向它发送确认。无效报告仍失败并关闭连接。
- 发布及回滚在激活 Host 后，检查其节点 ID 和具体 generation 已在目标 Server 在线、完成索引同步并就绪，再切换 Nginx；等待失败不切向该候选。既有等待与公共验收门槛不变。

## 功能与测试

| 功能 | 测试名 | 文件 |
|---|---|---|
| 写锁下心跳、报告顺序与提交后确认 | `locked_report_storage_keeps_ping_live_without_marking_index_synced` | [report_storage.rs](../../../crates/control/tests/node_keepalive/report_storage.rs) |
| 同代次重连不受旧完成影响 | `disconnected_report_cannot_mark_a_reconnected_generation_ready` | [report_storage.rs](../../../crates/control/tests/node_keepalive/report_storage.rs) |
| 失败报告不确认、不就绪 | `invalid_report_closes_without_handoff_or_ready_promotion` | [report_storage.rs](../../../crates/control/tests/node_keepalive/report_storage.rs) |
| 入站队列满仍可确认并保持帧序 | `full_incoming_queue_preserves_order_without_blocking_handoff_write` | [socket_io/tests.rs](../../../crates/control/src/transport/socket_io/tests.rs) |
| 出站 RPC 队列满不堵交接 | `full_rpc_queue_does_not_block_initial_host_handoff` | [socket_report/tests.rs](../../../crates/control/src/transport/socket_report/tests.rs) |
| 发布和回滚在确认新代次后切入口，失败保留原入口 | `test_forward_switch_waits_for_the_activated_host_generation`、`test_rollback_switch_waits_for_the_activated_host_generation`、`test_forward_host_sync_failure_preserves_old_ingress`、`test_rollback_host_sync_failure_never_changes_ingress` | [test_deployment.py](../../../scripts/platform/rolling_tests/test_deployment.py) |
| 其他节点、旧代次或未完成索引均不能替代新 Host | `test_activated_host_waits_for_its_generation_and_durable_ready_index` | [test_probes.py](../../../scripts/platform/rolling_tests/test_probes.py) |

## 验证

- 旧部署实现：新增发布与回滚次序测试均失败，检查新代次的调用数为 0；旧 socket 实现的真实索引写锁测试失败。
- 入站阻塞 enqueue 版本：队列满回归 1 failed；恢复有界 pending + reserve 后 1 passed。
- `cargo test -p opencoder-control --lib transport::`：29 passed。
- `cargo test -p opencoder-control --test node_keepalive --test release_handoff`：4 + 5 passed。
- `python3 -m unittest scripts.platform.rolling_tests.test_probes scripts.platform.rolling_tests.test_deployment`：43 passed。
- `cargo clippy -p opencoder-control --all-targets -- -D warnings`：零警告。
- 本次只运行改动及直接影响链路的定向测试；真实双节点发布、回滚与 15 分钟观察由发布作业另行验收。

相关：[Control](../../../agents/control/index.md)、[Server](../../../agents/server/index.md)、[调度平台](../../agent-platform/index.md)。
