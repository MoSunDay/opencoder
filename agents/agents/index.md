Commit: c139569c708859863cadc589ce49a6dcaf371f13

# agents 模块

版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。

## 索引
- `crates/agents/src/` — 池管理、引用卡、NFS 导出
- `crates/agents/src/references.rs` — memory 引用扫描
- [snapshot/files.rs](../../crates/agents/src/snapshot/files.rs) — 复制冻结资源时保留普通文件权限，Unix 去除特殊权限位；Tool 脚本的执行权限随快照保留。
- [snapshot](../../crates/agents/src/snapshot/mod.rs) — 固定资源当前版本；普通会话保留全部已注册 Agent 卡及其引用的资源，支持后续切换身份，不扫描未被引用的资源池。DAG 按所选 Agent 固定依赖。拒绝软链接与特殊文件，流式校验完整目录摘要，不复制无关历史版本。私有暂存树逐文件落盘，填充后的目录自底向上同步，共享类别目录在所有复制任务结束后同步，最后原子发布；已发布快照重试时保持不变。回归见 [snapshot/tests.rs](../../crates/agents/src/snapshot/tests.rs)。
- [nfs/acl.rs](../../crates/agents/src/nfs/acl.rs)、[serve/transport.rs](../../crates/agents/src/serve/transport.rs) — 只读 NFS 的 Linux ACL 查询与端口注册；ACL 写入始终拒绝，支持源工作区的内核写时复制。ACL 与普通 NFS 请求共享同一份路径注册表，已通过 LOOKUP 返回的长路径句柄不重新扫描导出目录；复制导出器时保留该共享关系。
- [nfs/handles.rs](../../crates/agents/src/nfs/handles.rs) — 短句柄直接校验相对路径，长句柄只查询共享注册表；未知长句柄返回 `STALE`，不访问导出树。导出器重启后，客户端必须重新 LOOKUP 长路径，才可继续使用对应句柄；格式错误仍返回 `BADHANDLE`。回归见 [handle_recovery.rs](../../crates/agents/src/nfs/tests/handle_recovery.rs) 和 [nfs_acl.rs](../../crates/agents/tests/nfs_acl.rs)。
- [testutil.rs](../../crates/agents/src/testutil.rs) — 资源追加、引用扫描及池管理测试共用全局资源目录锁，持锁期间使用独立临时目录。

## 相关
- [agents/core](../core/index.md) — 引用卡与资源根类型
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)

[write/deletion.rs](../../crates/agents/src/write/deletion.rs) 提供 `LockedAgentDeletion`：读取卡片前取得资源写锁，调用方可在锁内异步核对目录引用，随后在阻塞线程删除。只删除卡片目录，共享资源池保留；严格读取拒绝损坏文件和软链接。Server 的目录保护层负责引用与条件请求检查。
