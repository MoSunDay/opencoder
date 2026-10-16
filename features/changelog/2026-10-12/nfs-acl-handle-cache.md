Commit: 253aecf4ef9d88e32e99ebe81ba8de598eb925ad

# NFS 权限查询共享路径缓存

通过 OverlayFS 修改深层文件时，ACL 入口未共享普通 NFS 请求已登记的路径，导致长句柄解析扫描整个源工作区，出现超时、EIO 和后续 stale handle。导出器复制共享路径注册表，普通请求与 ACL 入口使用同一实例状态；保留只读、旧句柄恢复及路径安全约束。

| 行为 | 测试 |
| --- | --- |
| ACL 副本读取后续登记的长路径，解析不扫描源树，删除后读取失败 | `nfs_shared_handles::acl_clone_resolves_backend_handles_without_scanning_the_export` |
| 旧长句柄恢复、分页和路径安全 | `nfs::tests::handle_recovery::*` |
| TCP 监听、关闭和配置 | `serve::tests::*` |

定向测试、Clippy 与真实执行证据记录在仓库外的 `artifacts/acceptance/nfs-*`；不运行全量测试。上线验证必须包含真实容器内的深层文件修改和 NFS 权限查询。
