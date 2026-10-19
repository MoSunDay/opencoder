Commit: 22ad6e74e838cd16c90b75f94283c1d9c2223b8b

# 历史读取与 Runtime 自动休眠

历史详情需要唤醒原 Runtime。此前后台每 5 秒检查任务是否空闲，没有考虑刚结束的读取，连续查看历史会反复唤醒、休眠；唤醒与回收等待可能耗尽已有读取时限。

Host 现在记录实际转发给 Runtime 的请求，在请求完成、取消或超时后保留 60 秒空闲等待。连续访问刷新该时间；自动回收在取得使用锁后再次核对 Host 身份和最近访问。任务归属、读取超时、队列和进程检查保持原有约定；Host 直接读取持久化结果不唤醒 Runtime，也不延长其存活。无数据库迁移。

| 验证内容 | 定向用例 |
| --- | --- |
| 连续读取不触发自动休眠，访问停止后恢复回收资格 | `host::tests::hibernation::recently_read_retired_runtime_is_not_collected_between_history_requests` |
| 空闲时间到期、不同 Runtime 互不影响 | `host::activity::tests::grace_expires_and_later_reads_extend_only_their_runtime` |
| 完成或取消后重新计算完整空闲时间 | `host::activity::tests::completion_or_cancellation_starts_a_full_idle_period` |
| 原有转发时限、休眠准入、执行归属和并行版本 | `cargo test -p opencoder-agent host::` 的直接影响范围 |

新增历史读取回归在旧实现上失败，修复后通过。Host 定向测试通过 22 项；其中一项需要专用旧版本二进制的跨版本用例保持原有忽略状态，未计为通过。补充空闲到期断言后单独重跑该回归通过。`cargo clippy -p opencoder-agent --all-targets -- -D warnings` 通过，未运行全量测试。发布与稳定观察结果单独验收。

- [Agent 模块](../../../agents/agent/index.md)
- [运行历史](../../brain/index.md)
