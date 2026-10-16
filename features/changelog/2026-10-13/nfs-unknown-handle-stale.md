Commit: 84c013e1afb63028f7187bd9fb5250cd2425b8a4

# 未知 NFS 长句柄直接返回 STALE

移除未知长句柄的递归目录恢复。导出树包含阻塞的嵌套 NFS 时，该扫描会卡住整个导出器。长句柄只通过共享路径注册表解析；未登记或摘要不匹配时返回 `STALE`，无需读取导出目录。短句柄和格式错误校验不变。

导出器重启会清空长句柄注册表。客户端必须重新 LOOKUP 对应路径，恢复后路径 ID 和句柄字节保持稳定；不承诺缓存长句柄可直接跨进程重启使用。本改动不消除客户端正常请求某个不可用 NFS 路径时的阻塞。

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 未知句柄和已知 ID 的错误摘要均不打开或读取导出目录 | `unknown_opaque_handle_does_not_open_or_read_the_export_directory` | [handle_recovery.rs](../../../crates/agents/src/nfs/tests/handle_recovery.rs) |
| 导出目录不可用时未知句柄仍返回 STALE | `unknown_opaque_handle_is_stale_when_the_export_is_unavailable` | [handle_recovery.rs](../../../crates/agents/src/nfs/tests/handle_recovery.rs) |
| 重启后 fresh LOOKUP、稳定分页及完整资源读取 | `long_directory_pagination_recovers_after_restart_with_fresh_lookup`、`complete_skill_package_survives_restart_with_fresh_lookup` | [handle_recovery.rs](../../../crates/agents/src/nfs/tests/handle_recovery.rs) |
| 真 TCP NFS/ACL 的 STALE、fresh LOOKUP、READ 和 BADHANDLE | `old_opaque_handle_is_stale_until_fresh_lookup_and_shared_acl_can_read_it` | [nfs_acl.rs](../../../crates/agents/tests/nfs_acl.rs) |
| ACL 副本解析共享的已知长句柄，无需扫描 | `acl_clone_resolves_backend_handles_without_scanning_the_export` | [nfs_shared_handles.rs](../../../crates/agents/tests/nfs_shared_handles.rs) |

旧实现红测：未知句柄 2 项失败，真实 TCP 句柄回归 1 项失败。定向回归：`cargo test -p opencoder-agents`，56 passed / 0 failed，2 项既有手工测试未执行；未运行全 workspace 测试。相关 crate 执行 Clippy 与格式检查。真实容器、资源服务恢复和观察由发布验收另行记录。
