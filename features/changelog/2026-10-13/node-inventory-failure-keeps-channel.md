Commit: 3a58ad71918d7ea2fc0d5b19edf4e511acacac7d

# 清单读取失败时保留节点控制连接

项目页签和评论版本在本机切换后，观察阶段出现一次原生任务受理 503。Host 日志同时记录旧 Runtime 的 `/inventory` 读取失败；原实现通过 `prepared??` 退出整个 WebSocket，造成新任务受理短暂不可用。失败证据与原执行 ID 保留，重试不创建替代执行。

清单读取失败现在保留心跳和 RPC，发布 `ready=false`、实际错误及原负载计数。不能将旧清单当成新读到的内容，也不能发送空的完整索引报告。下一次心跳或真实修订重新读取；成功后按原顺序发布完整负载、索引和 Brain 信息。失败不推进 report ID。协议、调度就绪条件、容量预留、旧 Runtime 归属和传输错误处理保持原约定。

功能 → 定向测试：

- 两次清单读取失败后仍能 Inspect、保持真实 WebSocket 心跳、读取恢复后用原 report ID 同步真实执行 → `inventory_failure_keeps_calls_and_heartbeats_then_retries_a_complete_report`，位于 [report_failure.rs](../../../crates/node/src/fleet/client/tests/report_failure.rs)。
- 清单等待和准入等待同时发生时心跳独立运行 → `connection_heartbeats_while_admission_and_inventory_are_blocked`。
- 只读 Inspect 不触发全量清单、关闭取消正在读取的清单、准入与读取锁的顺序 → [client/tests.rs](../../../crates/node/src/fleet/client/tests.rs) 中的相关用例。

原实现红测真实复现连接被关闭。正式发布另核验切换期间的真实 TODO、原生任务、SSE、最大值 30 秒门槛与完整 15 分钟观察，不以本地测试代替发布验收。

相关说明：[节点传输](../../../agents/node/index.md)、[项目页签与评论](project-tabs-and-comments.md)、[平滑发布](../../../docs/smooth-release.md)。
