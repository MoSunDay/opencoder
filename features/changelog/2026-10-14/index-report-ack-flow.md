Commit: 5609d174d4eed9313e3e53723009de3cb1e77ab2

# 大型索引报告的背压与控制链

完整库存的数据库写入等待时，后续完整报告可填满入站队列，使活跃节点超时断线，任务详情返回 503。新 Node/Server 协商逐帧确认，提交完成前合并后续修订；完整报告提交由连接拥有的单个任务等待，原 Socket 继续读取取消回复与负载更新。旧 Node/Server 保持协议 10 和原 JSON，Brain 帧顺序、归属、索引水位、初始 pending 捕获与交接栅栏保持。

| 能力 | 定向测试 |
|---|---|
| 33,000 条报告、22 秒真实数据库写锁不阻断心跳/原调用取消回复，且报告不堆积 | `slow_full_report_commit_preserves_heartbeat_and_owned_cancel_reply` |
| 新确认报告断线后不得推进同代次重连的就绪或发送迟到确认 | `acknowledged_report_detach_cannot_promote_or_ack_replacement_connection` |
| 协商确认时 Brain 顺序、RPC、心跳保持 | `acknowledged_publication_preserves_brain_order_rpc_and_heartbeat` |
| 旧发送模式的分批传输、Brain 顺序、RPC、心跳保持 | `backpressured_publication_services_rpc_and_heartbeat_before_report_end` |
| 旧报告 JSON 完全一致；新确认标记显式启用 | `legacy_report_wire_format_stays_identical_and_new_flow_is_explicit` |
| 原锁等待、断线栅栏、无效报告、水位和无库存心跳约定 | `node_keepalive` 中原有 5 个测试 |

本轮先保留 ACK-only 方案下取消回复 504 的失败，再由连接拥有的提交任务验证成功。线上发布按 Server 信号完成；逐节点 900 秒任务与目标 Host 就绪观察、公共入口和实际 runc 取消证据单独记录。此前本节点 503 与 900 秒失败保持，不改写为从未失败。
