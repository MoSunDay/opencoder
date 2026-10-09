# DAG 清理等待磁盘写回

`runc delete` 超过 5 秒时记录慢清理并等待实际退出，继续保留运行归属。确认容器状态删除后才完成清理；真实退出失败仍重试一次，第二次失败保留状态并报错。

修复来自生产同一文件系统的平滑发布演练：`dag-traffic-46` 的步骤已成功，清理进程被固定超时结束后整批被记为失败。

功能与测试：

- 慢清理期间保留进程、状态和等待者，完成后再回收：`slow_delete_keeps_ownership_until_reaped_and_removed`。
- 真实退出失败后重试同一容器：`failed_delete_retries_same_owned_state_before_success`。
- 两次真实失败保留归属，且删除命令均已回收：`failed_delete_retains_owned_state_after_both_reaped_attempts`。
