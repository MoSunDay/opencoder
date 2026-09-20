# 已完成资源快照的创建重试

冷资源准备超过 RPC 等待期限时，Runtime 仍可能完成并发布不可变快照，但尚未写入执行接收记录。此前，同一执行 ID 的重试再次等待资源复制名额；持续冷创建占满名额时，已经完成快照的请求也会反复超时。

现在在持有单个执行的准备锁、恢复原始请求后，依据同一资源路径判断是否还需要复制。已有快照的重试直接执行原有预检、排队与持久化接收；新快照继续受 4 个复制名额限制。Project 的按运行归档路径、普通执行和旧执行的资源路径共用纯路径计算函数。原请求冲突检查和不可变快照校验保持生效。

回归用例在持有全部 4 个复制名额的情况下，恢复一个已经冻结快照的待接收请求，同时验证参数冲突仍被拒绝、原始请求未改变。修复前用例超时，修复后 Worker 单元测试 100 通过、0 失败、1 个已有忽略项。

发布与线上验收结果另记发布回执；本文件不表示修复已上线。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 已冻结快照绕过冷复制占用，拒绝变更请求 | `prepared_snapshot_retry_bypasses_occupied_copy_slots` | `crates/worker/src/operations/admission/tests/pinned_retry.rs` |
| 新冷创建保持限流，WASI 和冻结接收不受阻 | `new_wasi_admission_and_freeze_bypass_cold_resource_waiters` | `crates/worker/src/operations/admission/tests.rs` |
| 重启恢复原请求与定义 | `interrupted_preparation_recovers_its_original_request_after_restart` | `crates/worker/src/operations/admission/tests.rs` |

全量回归、零警告检查和构建的实际结果写入同日发布回执。
