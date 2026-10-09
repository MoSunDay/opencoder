# DAG 清理等待磁盘写回

`runc delete` 超过 5 秒时记录慢清理并等待实际退出，继续保留运行归属。确认容器状态删除后才完成清理；真实退出失败仍重试一次，第二次失败保留状态并报错。

若删除命令已退出，但原容器 init 正在处理 SIGKILL 或内核退出流程，先按 PID 和启动时间确认归属，等待该进程消失后再重试。PID 被复用、普通运行中的进程和其他容器均不进入这段等待，也不额外发送信号。同盘演练已捕获 init 在 OverlayFS 写回期间处于退出状态，超过了 runc 自身的等待期限。

修复来自生产同一文件系统的平滑发布演练：`dag-traffic-46` 的步骤已成功，清理进程被固定超时结束后整批被记为失败。

功能与测试：

- 慢清理期间保留进程、状态和等待者，完成后再回收：`slow_delete_keeps_ownership_until_reaped_and_removed`。
- 真实退出失败后重试同一容器：`failed_delete_retries_same_owned_state_before_success`。
- 两次真实失败保留归属，且删除命令均已回收：`failed_delete_retains_owned_state_after_both_reaped_attempts`。
- 内核退出完成前保持等待：`exiting_init_is_owned_until_kernel_exit_not_command_deadline`。
- PID 复用和正常运行不误等待：`reused_pid_or_running_init_never_waits_or_signals`。
- 区分退出、待处理 SIGKILL 与普通 I/O：`kernel_exit_and_pending_kill_are_distinct_from_normal_io`。
- 容器身份不匹配保留状态并拒绝清理：`mismatched_container_identity_is_rejected`。
