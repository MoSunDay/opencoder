Commit: 86fa2372caffd649b5809d14ca410fb269dcb760

# 避免小型执行详情反复读取完整冻结配置

历史执行详情此前按需读取整个 `execution.json`，即使接口不需要其中的冻结 runtime、队列和事件。正常会话的完整链路验收发现 CPU 已下降，但查询 P95 未达标；独立临时 Runtime 的系统调用追踪确认，约 128 KiB 文件的一次读取等待约 99 毫秒。

Journal 启动加载和保存成功时提取 Inspect 所需的小型详情。单条序列化最多 8 KiB，总计最多 16 MiB、32,768 条；完整正文缓存调整为 48 MiB、128 条，两者的字节预算合计仍为 64 MiB，按实际分配容量计费。淘汰使用有序索引，不遍历全部缓存。

详情保存索引、请求、定义、注释、结果、错误与初始化状态，不保存冻结 runtime、队列和事件。超限详情仍读取完整记录，原始文件和接口内容不变。详情类型只供读取，不能作为执行记录写回。缓存命中核对文件身份，返回前核对修订号；保存替换旧的小型详情，删除和文件替换使缓存失效，并发修改后重新读取，数据库确认标记继续覆盖旧文件中的标记。

保存后的缓存身份来自实际写入的文件句柄，避免路径被替换后将原详情关联到其他文件。旧读取请求不能淘汰或覆盖新修订的缓存。

## 定向验证

| 行为 | 测试 |
|---|---|
| 新保存的详情避免重读完整冻结配置 | `fresh_saves_keep_small_details_current_without_loading_frozen_bodies` |
| 保存缓存绑定实际写入文件 | `published_stamp_tracks_the_written_inode_and_rejects_content_changes` |
| 旧读取不能覆盖新缓存 | `delayed_read_cannot_evict_or_replace_a_newer_saved_view` |
| 小详情避免正文读取、完整数据仍可读取 | `loaded_small_details_do_not_read_or_cache_the_frozen_body` |
| 保存、删除及文件替换 | `save_delete_and_external_replacement_invalidate_small_details` |
| 大详情回到完整读取且内容不变 | `oversized_details_fall_back_without_changing_the_response` |
| 字节、条数上限 | `detail_cache_enforces_both_payload_and_entry_limits` |
| 链接与归属变化拒绝 | `cached_details_still_reject_symlinks_and_replaced_identity` |
| 当前确认标记覆盖 | `cached_details_apply_the_current_durable_ack_overlay` |
| 并发读取、保存和删除 | `concurrent_read_retries_save_and_observes_forget_without_holding_journal_lock` |
| 大定义与消息分页 | `oversized_legacy_message_is_chunked_and_detail_stays_bounded` |

测试代码见 [详情缓存](../../../crates/worker/tests/journal_memory/inspection.rs)、[并发与内存](../../../crates/worker/tests/journal_memory/cases.rs) 和 [分页](../../../crates/worker/tests/query_pagination.rs)。

- Worker 库测试：`cargo test -p opencoder-worker --lib journal::` 29 项、`operations::query::tests` 17 项通过，覆盖保存、恢复、确认、并发读取及详情接口。
- `cargo test -p opencoder-worker --test query_pagination`：1 项通过。
- `cargo clippy -p opencoder-worker --all-targets -- -D warnings`：零警告。
- 完整链路 CPU、查询延迟和内存以最终 release 程序的独立验收结果为准。之前未达标的样本保留，不计为通过。
- 验收脚本新增固定负载的 160 项任务吞吐对比，使用本地模型服务并核验全部落盘快照。CPU 与吞吐脚本的 6 项判定测试通过。三轮吞吐汇总保留全部样本，拒绝缺失轮次、程序变化或任务不足。
- 未运行全量测试，未操作生产服务。

相关说明：[Journal](../../../agents/worker/journal.md)、[完整链路 CPU 验收](../../../scripts/acceptance/cpu_inventory/README.md)。

## 验收中发现的活动任务读取问题

首个详情缓存候选通过完整链路 CPU、查询 P95、源轮询和三轮吞吐比较，但 80 项任务后最后 20 项 RSS 增长 25.6 MiB，超过 16 MiB 门槛，总 RSS 仍低于 256 MiB。该失败保留。保存时直接更新小型详情，避免活动任务在刚保存后重新读取和解析完整冻结配置。后续诊断发现大量默认 Tokio 工作线程进一步放大分配器保留内存；收敛默认线程数后，最终候选的 80 项任务、4 GiB 历史、吞吐、完整链路 CPU 与查询延迟均通过。结果与测试边界见 [Runtime 工作线程验收](runtime-executor-memory.md)。
