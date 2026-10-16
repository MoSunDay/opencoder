Commit: 2cc50a8f6d00b684090404b1778113d2a736d9aa

# 取消保存后立即发送进程信号

取消曾在保存 stop_intent 后再次等待 journal 锁，库存或调度读取可能把信号延迟到 Host 的 10 秒命令转发窗口之后。停止意图与停止中的 TODO 重跑状态现在共用一次 journal guard，随后立即发出信号；不改变归属、额度、知识库能力或清理完成才能释放的要求。

`durable_stop` 沿用 250ms 慢请求门槛，日志仅记录执行 ID、生命周期锁、活动执行查找、journal 锁、停止落盘、重跑状态和信号耗时；它不覆盖进入 `durable_stop` 前的控制链等待。历史 504 与两端 21d 验收记录保留，改进不把历史失败改写成成功。

定向验证：`durable_cancel_signals_before_a_queued_journal_reader_can_block_it`、`native_cancel_reaps_runc_and_descendants_without_model_calls`。相关：[worker](../../../agents/worker/index.md)、[执行平台](../../agent-platform/index.md)。

发布备份兼容任务过程文件：Runtime 任务 HOME/WORKSPACE 中带 `.db` 后缀的非 SQLite 文件按源文件在复制期间未变化的字节副本保留，权限和摘要记录入 manifest；不打开表、不修改原件。平台 DB 和带 SQLite 文件头的损坏 DB 继续严格失败，不把用户过程存储误当平台 SQLite，也不漏备份这些文件。定向验证见 `rolling_tests/backup/test_snapshot.py`。
