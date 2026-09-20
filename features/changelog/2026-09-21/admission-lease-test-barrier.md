# 等待预检租约完整释放后断言

全量回归中，`cancelled_preflight_holds_execution_and_copy_leases_until_io_finishes` 在获取生命周期锁后偶发读到复制名额仍为 0。租约是一个元组，其析构依次释放互斥锁和信号量；等待互斥锁的任务可能在第二个字段析构前被唤醒。

测试现在分别等待生命周期锁和复制名额释放，再核验最终名额。阻塞 I/O 完成前的两项占用断言、取消传播和两个 1 秒死锁检测期限均保留，生产逻辑未改变。

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 调用取消后 I/O 继续持有两种租约，结束后全部释放 | `cancelled_preflight_holds_execution_and_copy_leases_until_io_finishes` | `crates/worker/src/operations/admission/tests.rs` |

原始失败：`/var/tmp/opencoder-latest-release-20260920/final5e7c-tests.log`。修正后的全量结果和发布版本以最终回执为准。
