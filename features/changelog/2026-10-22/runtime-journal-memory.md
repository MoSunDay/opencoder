Commit: f55dfb3672ed7ff227a3cd3e900e09dced7065c3

# 降低 Runtime 历史执行记录的内存占用

## 原因与范围

旧 Runtime 的只读采样显示，约 37.3 GiB 堆空间中，约 34.7 GiB 是已释放但分配器仍保留的空间。周期性复制完整 journal 会反复分配这些大对象；CPU 专项已处理这条复制路径，但完整历史及重复的冻结配置仍长期驻留。

本次把完整历史从常驻对象移到原始文件按需读取。内存保留执行摘要和必要的上报标记；启动、恢复和队列扫描逐条处理。读取缓存只存序列化字节，最多 64 MiB、128 条，超限单条不缓存；最多同时执行 4 次阻塞读取与解析。保存采用缓冲写入，避免额外构造整条序列化副本。

缓存直接复用文件读取的 `Vec` 缓冲区，避免转换成 `Arc<[u8]>` 时再复制整块内容；字节预算按实际分配容量核算，包含预留空间。这样减少大块临时分配与分配器保留的空间，不改变任务数据和缓存上限。

普通查询返回前检查修订号，遇到并发保存或删除重新读取。成功保存和删除使缓存失效，缓存命中也检查文件身份；已有记录丢失、损坏或变成链接必须报错。Brain 确认写回加载完整记录并保留原始数据；待确认派发最多复用一个根运行的同一意图版本，队列清空时释放。

磁盘格式、任务身份、冻结配置、生命周期锁、恢复与取消规则保持兼容。没有清理历史、操作生产鉴权数据、主动回收分配器或发布重启线上进程。

## 实际内存验收

使用真实 Runtime 和合成历史，每条历史包含冻结 runtime 与定义快照。基线已经包含 CPU 候选上报优化，空上报约 2 毫秒。每个场景在同一个进程中连续进行 5 轮上报与查询，每轮并发读取 256 条历史。

| 历史量 | 记录数 | 基线最终 RSS | 修复实现最终 RSS | 最后两轮变化 |
|---|---:|---:|---:|---:|
| 0 | 0 | 62.20 MiB | 69.76 MiB | 0 MiB |
| 1 GiB | 4,096 | 1,121.13 MiB | 138.73 MiB | 2.188 MiB |
| 4 GiB | 16,384 | 4,235.70 MiB | 159.05 MiB | 0.156 MiB |

1 GiB 与 4 GiB 场景分别下降约 88% 和 96%；相对空历史增加约 69 MiB、89 MiB，均低于 256 MiB。修复实现的查询 P95 分别为 32.98、55.67 毫秒，所有查询返回正确结果；最大单次查询为 204.95 毫秒。启动峰值分别约 81、100 MiB，整个查询场景峰值不超过表中最终 RSS。

另连续执行 5 批、每批 16 个真实任务，以本地模型固定回答结束对话后持久取消。每个任务携带 1 MiB runtime 数据，原 runtime 和排队配置中的测试数据均完整保留。80 个任务后，修复实现约 275 MiB、峰值约 294 MiB，最后两轮相差 1.188 MiB；基线约 350 MiB，最后两轮仍增加约 50 MiB。两个版本各 80 条落盘记录的状态、停止意图及两份配置中的 1 MiB 测试数据均逐条核对通过。

测试过程中没有在轮次之间重启进程，也没有调用分配器回收函数。数值来自合成负载，不代表线上进程已经生效。测量构建 SHA-256：基线 `7b5b4315cb92959771f94215bf37f32b7d6fdc62c58e3290fc3e7010ac38d2d5`；按需读取实现 `72b845aec3f74c3008d83248ceaa5237104427d1d08a38cff772c88d7f3647ab`。随后对并行 CPU 改动新增的单个派发缓存补充了队列清空释放和定向测试。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 历史按需读取与原始内容保留 | `history_stays_on_disk_and_cold_reads_keep_complete_records` | [cases.rs](../../../crates/worker/tests/journal_memory/cases.rs) |
| 缓存字节、条数、淘汰与大记录 | `cache_is_bounded_by_bytes_and_count_and_large_records_bypass_it`、`lru_evicts_the_least_recent_entry_and_enforces_the_byte_budget` | [cases.rs](../../../crates/worker/tests/journal_memory/cases.rs)、[cache.rs](../../../crates/worker/src/journal/cache.rs) |
| 复用读取缓冲区、预留容量计费 | `spare_buffer_capacity_is_charged_without_copying_the_allocation` | [cache.rs](../../../crates/worker/src/journal/cache.rs) |
| 并发保存、删除和读取 | `concurrent_read_retries_save_and_observes_forget_without_holding_journal_lock` | [cases.rs](../../../crates/worker/tests/journal_memory/cases.rs) |
| 缺失、损坏、链接和写失败 | `warm_cache_does_not_hide_missing_corrupt_or_replaced_records`、`failed_save_preserves_summary_cache_and_report_state` | [cases.rs](../../../crates/worker/tests/journal_memory/cases.rs) |
| 确认保存不覆盖完整历史 | `report_ack_preserves_full_payload_in_both_persistence_modes` | [cases.rs](../../../crates/worker/tests/journal_memory/cases.rs) |
| 历史派发数据释放 | `historical_brain_intents_are_not_resident_report_state`、`empty_outbox_releases_the_last_parsed_dispatch_without_another_frame_poll` | [cases.rs](../../../crates/worker/tests/journal_memory/cases.rs)、[dispatch.rs](../../../crates/worker/tests/journal_memory/dispatch.rs) |
| 真实 Runtime 内存与任务完成 | 0/1/4 GiB 历史、连续 80 个任务 | [验收脚本](../../../scripts/acceptance/runtime_memory/README.md) |

- `cargo test -p opencoder-worker --lib journal::`：19 passed。
- `cargo test -p opencoder-worker --lib brain::outbox::`：2 passed。
- `operations::query::`、`operations::admission::`、`operations::todo::control::tests`、`operations::create_retry_tests`：分别 27、9、4、2 passed；受理测试包含真实 runc 取消清理。
- 15 个集成测试目标共 48 passed：`brain_nested`、`brain_outbox_candidates`、`brain_report`、`brain_report_stability`、`conversations`，以及 `durable_execution`、`durable_lifecycle`、`layout_migration`、`query_pagination`、`project_replay`、`initial_input_recovery`、`operator_initialization_recovery`、`resource_snapshot`、`queue_project`、`dag_image_recovery`。
- `cargo test -p opencoder-worker --test brain_io recovery:: -- --test-threads=1`：2 passed、11 filtered out。以上合计 63 个库测试、50 个集成测试，均通过。
- `cargo clippy -p opencoder-worker -p opencoder-agent --all-targets -- -D warnings`：零警告。
- 历史对比与任务完成两组内存验收均为 PASS；过程记录保留在 `/tmp/opencoder-memory-fix-20261022/`。
- 测试范围仅覆盖本次修改及直接影响链路，未运行全量测试，未发布。

DAG 镜像恢复测试先后遇到跨挂载路径的硬链接失败和内核挂载锁等待。调整测试镜像为同一底层挂载路径，等待挂载阻塞消退后，原用例串行通过；没有修改测试断言或超时。后 10 个集成目标直接运行本轮 Cargo 已构建的测试程序，逐个保存退出码和测试摘要。

## 最终发布候选验证

最终程序提交为 `f55dfb3672ed7ff227a3cd3e900e09dced7065c3`，Agent SHA-256 为 `2ee8d81dc30b75c25368081b2ef84f1f56ade0d0b7764b13c2fb46c03e4e2e5a`。七个程序均来自同一干净源码提交，前端资源与当前部署版本一致。此前 journal 定向回归 20 项及相关测试继续适用：最终候选的 Worker 源码与依赖没有再变化；本轮另外验证了报告协议和交接路径。

| 最终发布构建场景 | 实测结果 |
| --- | --- |
| 空历史 RSS | 28.30 MiB |
| 4 GiB 历史、16,384 条完整记录、5 轮读取 | 最终 RSS 96.87 MiB，峰值 106.11 MiB |
| 历史查询最大耗时 | 65.06 毫秒 |
| 同一进程连续 80 个任务 | 最终 RSS 196.80 MiB，峰值 201.25 MiB |
| 最后 20 个任务的 RSS 变化 | 增加 12.97 MiB，低于 16 MiB 验收上限 |
| 完整落盘内容 | 80 条记录逐条验证通过 |

测量过程没有重启或分配器强制回收。发布构建基线 `d0a0eda8` 的 Agent 源码与当前部署 `8a4025fa` 相同，已经包含早期按需读取修复；相同负载下历史 RSS 为 100.61 MiB，80 个任务后为 216.89 MiB。早期数 GiB 的复现来自不同构建阶段，不能拿它与最终发布构建直接计算性能下降比例。

最终候选的跨版本 Schema/ACK 验证通过。实际旧版本到候选的交接曾出现一次 503，随后补充生产者先升级、增量协议 2、完整报告 Brain 确认和 Host 管理入口修复。最终包混合版本演练的 18 项检查通过，208 次连续提交零失败；所有样本结束后，候选与回滚版本各完成 900 秒观察，分别完成 53、54 个任务，Runtime 未重启。测试数据与线上 Runtime 位于同一块 ext4 磁盘。详见[交接修复](host-first-report-upgrade.md)。

候选包、保护回滚包和发布控制器已分别固定摘要并完成归档回读，上线预检通过，达到本轮上线准备标准。当前部署基线已经包含早期内存修复，不能把最终候选的差异描述为从数 GiB 再下降到百 MiB。仅执行隔离验收，未激活生产版本；实际发布后的整机内存变化仍需结合保留 Runtime 的任务结束与休眠观察。

## 相关说明

- [执行记录的内存与持久化](../../../agents/worker/journal.md)
- [Agent 平台](../../agent-platform/index.md)
- [CPU 候选上报优化](cpu-index-sync.md)
