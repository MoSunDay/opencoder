Commit: 522cd534c427cec2a55d51ea6955c0774a90a63b

# Host 容量轮询避免无效写入

Dynamic DAG 全工作区回归中，三版本 Host FIFO 测试曾持续等待超过十分钟。容量已满时，调度器每次重放 ticket 仍执行 `INSERT ... ON CONFLICT DO NOTHING`，递增 SQLite 自增序号；领取轮询也无条件开启写事务，争用其他运行时接收任务所需的写锁。

已入队 ticket 现在直接读取并检查归属，容量不足或未到队首时只读返回。真正领取时仍在事务内重新检查相同的 FIFO 和容量条件，保持跨连接的原子性。未修改容量、超时或调度顺序，未新增表或应用环境变量。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 其他连接持有写锁时重放 ticket，保留归属校验 | `repeated_capacity_ticket_is_read_only_under_writer_contention` | [Store 回归](../../../crates/store/src/fleet/handoff/capacity_tests.rs) |
| 满容量轮询只读，释放后正常领取且不可重复领取 | `full_capacity_poll_is_read_only_under_writer_contention` | [Store 回归](../../../crates/store/src/fleet/handoff/capacity_tests.rs) |
| 三版本共享容量、回滚与 FIFO | `ownership_and_fifo_capacity_span_three_releases_and_rollback` | [handoff](../../../crates/store/tests/handoff.rs) |
| 真实模型调用及三版本 Host FIFO | `three_runtime_versions_keep_live_model_calls_and_global_fifo` | [Host](../../../crates/agent/src/host/tests.rs) |

两个新增测试通过持有独立连接的写事务检测无效写入，旧实现均失败（`/tmp/dynamic-capacity-before.log`）。修复后新增 2 项及既有 handoff 5 项通过（`/tmp/dynamic-capacity-after.log`）；三版本 Host 测试 19.07 秒通过（`/tmp/dynamic-capacity-agent.log`）。全工作区最终结果见 [Dynamic DAG 验收](dag-dynamic-instances.md)。
