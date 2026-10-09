Commit: 1374d7ed231300bd2d00e13790c6f94da3ac9812

# 容器初始化进程退出与清理归属

runc 删除可能先于内核写回结束而失败。DAG 清理核对容器及进程启动身份，等待已进入退出流程的原进程，再重试删除；状态目录与挂载仍须真正清理完成才能交付终态和释放容量。相关实现见 [dag-runtime](../../../agents/dag-runtime/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 慢退出期间保留归属 | `exiting_init_is_owned_until_kernel_exit_not_command_deadline` | `crates/dag-runtime/src/sandbox/runc/init_exit.rs` |
| PID 复用及正常进程 | `reused_pid_or_running_init_never_waits_or_signals` | 同上 |
| 内核退出与待处理终止信号 | `kernel_exit_and_pending_kill_are_distinct_from_normal_io` | 同上 |
| 容器身份不匹配 | `mismatched_container_identity_is_rejected` | 同上 |

- `cargo test --workspace --no-fail-fast -- --test-threads=4`：5747 passed、0 failed、8 个既有 manual/ignored。项目执行测试使用独立知识目录，避免读到开发机全局配置。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告；workspace 构建通过，开发候选七个二进制与镜像运行器构建信息一致。
- 真实 runc 切换、回滚与 900 秒真实磁盘观察通过；受理耗时、连续受理间隔和调度间隔的最大值均低于 30 秒。
