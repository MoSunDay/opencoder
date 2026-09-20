# 清理 runc 创建被取消后留下的空状态目录

候选 `29372a2c` 的全量回归通过后，显式 runc 验收发现：取消可发生在 runc 创建私有目录、写入状态之前。启动进程退出后目录为空，`runc delete --force` 返回容器不存在，原先的清理流程因此失败并留下目录。

普通执行、启动恢复和进程监督共用空目录清理函数。它使用原子 `rmdir`，只接受真实目录；非空状态仍交给 runc，符号链接、异常路径及权限错误直接报错。原有取消、超时、输出限额、进程回收及容器状态消失断言均保留。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 创建中断、尚无容器元数据 | `cleanup_removes_interrupted_creation_without_container_metadata` | `crates/dag-runtime/src/sandbox/runc.rs` |
| 空目录及幂等清理 | `empty_creation_state_is_removed_and_missing_state_is_idempotent` | `crates/session/src/process/runc_state.rs` |
| 保留元数据和未知内容 | `metadata_and_unknown_entries_require_normal_container_cleanup` | 同上 |
| 拒绝符号链接 | `symlinks_are_rejected_without_touching_the_target` | 同上 |
| 真实容器取消与超时后无残留 | `cancellation_and_timeout_remove_running_containers` | `crates/dag-runtime/src/sandbox/runc.rs` |

失败原始回执保留在 `/var/tmp/opencoder-agent-menu-delivery-20260920/candidate-29372a2c/manual-runc-tests.log`。修正后的定向测试、全量回归和发布结果以最终候选回执为准，当前尚未发布。
