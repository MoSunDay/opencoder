Commit: 8f727cf0cf8c7cbf7e2331b381d3a3f194ec03d1

# 休眠版本清单不再探测旧端口

Host 原本在每次全量清单中先探测所有历史 Runtime 的 `/inventory`，失败后才读取休眠时保存的最终清单。旧端口可能被其他服务占用，或保留旧连接；慢响应拖住同步，成功响应还会清除休眠标记并覆盖冻结历史。本机检查发现 123 个休眠 Runtime，其中一个旧端口仍有监听程序。

已明确休眠的 retired Runtime 现在直接读取保存的最终清单，并保留执行归属与休眠标记。实际运行的版本继续实时读取；不可用仍报告未就绪。显式读取需要唤醒的历史执行时，仍沿用原 Host 准入锁、Runtime 使用锁、准入恢复与标记清除流程。只读分块结果保持原目录读取方式，不迁移执行，不启动或重启已有 Runtime。

功能 → 定向测试：

- 休眠后旧端口被占用，冻结历史、归属和标记仍保留，旧端口请求数为零 → `sleeping_inventory_does_not_probe_a_reused_endpoint_or_clear_its_marker`。
- 未休眠的活动 Runtime 不可用仍报告未就绪 → `unavailable_live_inventory_still_fails_readiness_without_a_sleep_marker`。
- 冻结/开放不唤醒休眠版本、显式唤醒继承当前准入、退休快照错误不阻断活动版本、使用锁保护回收 → [hibernation.rs](../../../crates/agent/src/host/tests/hibernation.rs)。
- 只读 RPC 与实际唤醒的修订通知 → [read_reports.rs](../../../crates/agent/src/host/tests/read_reports.rs)。
- 三版本继续保持原归属与全局 FIFO → `three_runtime_versions_keep_live_model_calls_and_global_fifo`。

原实现红测发现冻结的历史执行被实时空清单覆盖。正式发布继续保留原失败回执，并重新核验真实切换及完整 15 分钟观察。

相关说明：[Host/Runtime](../../../agents/agent/index.md)、[节点清单错误处理](node-inventory-failure-keeps-channel.md)、[平滑发布](../../../docs/smooth-release.md)。
