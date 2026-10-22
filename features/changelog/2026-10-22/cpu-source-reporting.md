Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# 源增量索引和按需上报

Runtime 只读取索引所需列，并用源事务内的变更记录更新缓存。Host 获取固定快照分页或源变化，Node 在 Server 确认后推进源游标。完整校准保留首次、重连、游标过期和每 10 分钟四种入口。

Journal 常驻摘要，正文按需读取并限制缓存。Brain 待发送队列与运行状态同事务提交，每次先选择最多 64 项再构造消息。终态 ACK 只使用摘要，独立确认表单调更新，重复 ACK 不写入；派发意图完成后释放，引导按序发送。引导队列只索引每个运行当前可发送的一条，确认删除与下一条开放同事务提交，积压消息不进入周期查询。

发布增加 `report_state` 读取/写入声明，先 1/0 后 1/1。格式检查覆盖回滚与保留 Runtime 的程序替换。

| 能力 | 定向验证 |
| --- | --- |
| 索引列隔离、变更、删除、回滚、子会话归属 | `store/tests/inventory_sources.rs` |
| 固定分页、游标过期、无变化读取 | `core::fleet::inventory::tests` |
| Runtime 分页期间源变化和后续删除同步 | `node_inventory_reads_more_than_500_sessions_with_activity_cursors` |
| Host 多页读取、空增量、删除、generation 变化 | `runtime_pages_flow_into_host_deltas_and_generation_resets` |
| Server 确认后推进游标、断线重新全量 | `producer_changes_advance_only_after_commit_and_reconnect_requires_full` |
| Brain 事务、顺序、恢复、确认零写入、限量解析 | `store/tests/brain_layered_v4/reporting.rs` |
| 终态轮转、ACK 不读取或改写正文、重启重放 | `terminal_candidates_rotate_past_sixty_four_and_acks_survive_restart` |
| 同代派发跨确认复用、新代失效、无关历史不进入轮询 | `journal::reporting::tests` |
| 引导队列迁移、使用当前消息索引、确认与推进原子提交 | `legacy_guidance_queue_migrates_to_indexed_heads_and_ack_is_atomic` |
| 读取过渡版本、降级拒绝、清单与二进制一致 | `rolling_tests/test_report_state.py` |
| 2 千、2 万、10 万条空闲及单条变化的局部比较 | `incremental_source_cost_does_not_grow_with_idle_history`（显式性能用例） |

已执行：索引存储 4 项、核心分页 2 项、Brain 存储 13 项、Node 同步 15 项、Python 发布兼容 17 项通过。Host 整组初次回归 18 通过、3 失败、1 条件忽略；报告锁用例独立通过，两项生命周期用例在修正终态清理等待并串行复跑后通过。Worker 首次单元回归 123 通过、3 失败；补齐匹配镜像并修正异步读盘的取消断言后，相关用例通过。构建和备份并行时另有两项受理/原生取消超时；降低 I/O 争用后原阈值通过，保留失败及复测记录。Brain 上报、人工引导和分层调度共 16 项集成测试通过，新增派发缓存测试通过。

第一版曾用含 22,694 条受理记录的控制库副本验证附加归属表回填，但该单进程测试未覆盖运行中旧进程的 SQL 缓存。实际预热增加共享表时，旧 Runtime 报 `Database schema changed` 并停止接受新任务。原发布失败证据保留；旧任务未重启。最终实现取消新增共享表，归属复用已有索引，休眠修订使用原 JSON 的固定长度前缀。新旧程序共库的跨进程回归验证结构版本不变。Journal 的 0/1/4 GiB 历史和连续 80 个任务验证见 [内存验收记录](runtime-journal-memory.md)。

局部基准使用实际 `DeltaState` 与 `InventoryLog`，比较前一阶段完整摘要重建和源增量路径（debug 构建，单进程 30 次采样，不含数据库、网络和模型）。

| 历史数 | 旧空闲 P95 | 新空闲 P95 | 旧单条变化 P95 | 新单条变化 P95 |
| --- | --- | --- | --- | --- |
| 2,000 | 9,858 μs | 23 μs | 9,411 μs | 28 μs |
| 20,000 | 111,377 μs | 20 μs | 106,730 μs | 42 μs |
| 100,000 | 685,500 μs | 28 μs | 584,555 μs | 64 μs |

该用例通过空闲成本不随历史显著增长及局部开销减少至少 50% 的断言。局部基准不代表线上 CPU。发布与线上观察尚未完成，不据此声明优化已在线上生效。

恢复检查只允许已归因到旧活动 Runtime 的就绪错误通过预热；入口切换前仍要求新 Host 代次和新 Runtime 实际就绪。长发布可显式增加作业等待时间，持续提交、SSE 和 900 秒观察门槛保持原值。新增验证：`shared_database_upgrade_preserves_the_running_old_connections`、`sleep_prefix_tracks_new_and_legacy_writes_without_schema_changes`、`test_standby_attributes_only_the_replaced_runtime_error`。

配置库同样使用独立的 `open_catalog` 路径，避免 Server 预热时创建 Runtime 专用扩展表。跨进程用例 `shared_catalog_does_not_install_runtime_reporting_schema` 验证旧连接保持可用，普通 Runtime 打开仍创建全部上报表。

结构兼容修正验证：Fleet 存储 30 项、Host 21 项通过（1 项旧结果格式条件用例忽略），发布恢复 30 项、预热检查 15 项、验收脚本 8 项通过。Clippy 的一处测试多余复制已修正，Store/Agent 定向检查通过。修正后的 2 万索引、500 MiB 冻结请求比较：旧查询 1,382 CPU tick，新路径 136 tick，减少约 90.2%；仍为隔离局部比较。
