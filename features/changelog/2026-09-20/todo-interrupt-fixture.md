Commit: 035e4d2eec3dc2bd15cd76901a1b5e1c5143f49f

# TODO 中断兼容接口的派发准备

Dynamic DAG 全工作区回归中，`todo_interrupt_compat_route_remains_resumable_once` 在等待 Server 索引的准备阶段超过 10 秒，中断与恢复断言尚未执行（`/tmp/dynamic-private-inventory-timeout.log`）。原测试直接向 Worker 创建任务，随后依赖异步库存报告为 Server 补齐索引；未修改代码的单项复测通过（`/tmp/dynamic-durable-recheck.log`）。

准备流程改为调用公开的 `POST /api/executions`，检查 202 回执，并立即断言 Server 索引的类型和节点归属。这样使用真实派发流程建立兼容接口所需的索引。模型调用、中断、恢复及重复恢复返回 409 的原有断言和超时保持不变，未修改生产逻辑。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| Server 派发、持久化索引、TODO 中断和仅一次恢复 | `todo_interrupt_compat_route_remains_resumable_once` | [durable execution](../../../crates/worker/tests/durable_execution/main.rs) |

修正后 `durable_execution` 在原 ext4 临时目录中 6 项全部通过，耗时 48.39 秒（`/tmp/dynamic-durable-fixture-after.log`）。工作区最终结果见 [Dynamic DAG 验收](dag-dynamic-instances.md)。
