Commit: c139569c708859863cadc589ce49a6dcaf371f13

# 历史读取与 Runtime 自动休眠

历史详情需要唤醒原 Runtime。此前后台每 5 秒检查任务是否空闲，没有考虑刚结束的读取，连续查看历史会反复唤醒、休眠；唤醒与回收等待可能耗尽已有读取时限。

Host 现在记录实际转发给 Runtime 的请求，在请求完成、取消或超时后保留 60 秒空闲等待。连续访问刷新该时间；自动回收在取得使用锁后再次核对 Host 身份和最近访问。任务归属、读取超时、队列和进程检查保持原有约定；Host 直接读取持久化结果不唤醒 Runtime，也不延长其存活。无数据库迁移。

| 验证内容 | 定向用例 |
| --- | --- |
| 连续读取不触发自动休眠，访问停止后恢复回收资格 | `host::tests::hibernation::recently_read_retired_runtime_is_not_collected_between_history_requests` |
| 空闲时间到期、不同 Runtime 互不影响 | `host::activity::tests::grace_expires_and_later_reads_extend_only_their_runtime` |
| 完成或取消后重新计算完整空闲时间 | `host::activity::tests::completion_or_cancellation_starts_a_full_idle_period` |
| 原有转发时限、休眠准入、执行归属和并行版本 | `cargo test -p opencoder-agent host::` 的直接影响范围 |

新增历史读取回归在旧实现上失败，修复后通过。Host 定向测试通过 22 项；其中一项需要专用旧版本二进制的跨版本用例保持原有忽略状态，未计为通过。补充空闲到期断言后单独重跑该回归通过。`cargo clippy -p opencoder-agent --all-targets -- -D warnings` 通过，未运行全量测试。

独立真实进程验收中，连续读取同一历史执行 30.25 秒，原 Runtime 进程不变；停止访问约 64.24 秒后自动休眠，重新打开同一执行能够唤醒并保留原数据。原有两轮六个子执行、44 项浏览器检查和真实产物下载也通过。

该修复版本在两套正式环境分别完成发布后的 900 秒观察，以及所有验收任务结束后的独立稳定性观察：本机 901.78 秒、远端 900.30 秒。远端持续读取固定的历史执行并核对其原 Runtime 进程，未再发生读取间反复休眠。早期发布等待超时和共享主机上的短时就绪检查超时均保留失败记录；后续完整复验使用原有可用性与响应门槛通过，共享主机延迟的具体等待位置仍未确认。后续整合维护的发布状态单独记录，这些窗口不替代新版本的最终验收。

- [Agent 模块](../../../agents/agent/index.md)
- [运行历史](../../brain/index.md)
