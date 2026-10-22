Commit: 86fa2372caffd649b5809d14ca410fb269dcb760

# 限制执行 Runtime 的默认异步工作线程数

64 核主机上的隔离运行发现，Runtime 默认创建 64 个 Tokio 工作线程，完成短任务后各线程的分配器保留了较多空闲内存。80 项任务后诊断得到 75 个内存分配区、约 87 MiB 空闲内存。设置 8 个工作线程的对比进程有 18 个分配区，最终 RSS 约 171.5 MiB，最后一批结束后未继续增长。此诊断没有回收分配器内存或在批次间重启进程。

Runtime、Run 和默认执行入口的工作线程现在取可用 CPU 数与 8 的较小值，显式 `TOKIO_WORKER_THREADS` 保留原生优先级及校验。Host 和离线命令保持原默认值；任务名额、DAG 并发、Brain 层规则及外部进程容量不变。没有新增配置项、协议字段或持久化格式。

## 验证范围

| 行为 | 测试 |
|---|---|
| 小主机随可用 CPU，大主机有上限，三个执行入口一致 | `execution_defaults_follow_small_hosts_and_bound_large_hosts` |
| 显式配置与 Host、内部进程监督入口保留原行为 | `explicit_thread_configuration_and_host_defaults_are_preserved` |

`cargo test -p opencoder-agent --bin opencoder-agent executor::`：2 项通过；`cargo clippy -p opencoder-agent --all-targets -- -D warnings`：零警告。同一 release 程序通过显式线程配置做了三轮对照，各完成并核验 480 项任务，汇总吞吐比值 1.139，满足不低于 0.95 的门槛。最终候选 `86fa2372` 的七个程序版本与摘要一致，独立 release 验收全部通过。已失败的旧候选不计为通过。未运行全量测试，未操作生产服务。

相关说明：[Agent 入口](../../../agents/agent/index.md)、[Journal 详情缓存](inspect-small-details.md)。

## 最终候选验收

- 同一 Runtime 连续完成 80 项任务，冻结快照全部正确；最终 RSS 172.2 MiB，最后 20 项增长 13.7 MiB，分别满足 256 MiB 和 16 MiB 门槛。
- 4 GiB 历史、连续五轮查询后 RSS 107.1 MiB；未主动回收分配器内存或在批次之间重启。
- 三轮完整链路对照中，Server、Host、Runtime 合计 CPU 的中位数从 42.09% 降到 5.34%，下降 87.3%（一个核心为 100%）；查询 P95 中位数从 4.43 降到 3.83 毫秒。1,440 次请求零错误，20,000 条历史的数量与归属核对一致。
- 三轮任务吞吐对照各完成并核验 480 项任务，汇总 8.04 → 13.53 项/秒，满足新版不低于旧版 95% 的门槛。单轮耗时有磁盘波动，汇总使用全部任务和全部批次耗时，不将该数值解释为生产环境的固定增幅。
- 2 千、2 万、10 万条历史的源轮询 CPU 与查询延迟门槛全部通过。
- `scripts/acceptance/runc_scheduling/main.py`：双节点 20 项原生执行样本、失败与超时、取消、3 个动态实例、16 MiB 产物和重启后的冻结版本恢复通过；测试进程正常退出，无遗留挂载，源工作区不变。

以上为隔离候选的 CPU 与直接影响链路验收。统一主工作区的发布及生产观察由既定发布流程完成，不能据此认定生产已生效。
