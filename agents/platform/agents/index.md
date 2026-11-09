Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# agents 模块

版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出
（lib.rs 写路径）。

## 索引
- [write.rs](../../../crates/agents/src/write.rs) — 版本与引用卡变更核心（`save_resource_version` 等原子写）
- [write/deletion.rs](../../../crates/agents/src/write/deletion.rs) — `LockedAgentDeletion`：从核对卡片到删除持资源写锁
- [rollback.rs](../../../crates/agents/src/rollback.rs) — `rollback_resource` 指针式回滚；版本目录从不删除
- [references.rs](../../../crates/agents/src/references.rs) — 引用扫描：池当前版本内容快照进卡片 `references`
- [snapshot/](../../../crates/agents/src/snapshot/mod.rs) — 冻结资源当前版本的快照（files.rs 复制规则）
- [nfs.rs](../../../crates/agents/src/nfs.rs)、[nfs/handles.rs](../../../crates/agents/src/nfs/handles.rs) — agents 根上的只读 NFSv3 与有界句柄
- [nfs/acl.rs](../../../crates/agents/src/nfs/acl.rs) — 只读 NFS 的 Linux ACL 查询
- [serve.rs](../../../crates/agents/src/serve.rs)、[serve/transport.rs](../../../crates/agents/src/serve/transport.rs) — NFSv3 服务与端口注册（单 TCP 监听）
- [io.rs](../../../crates/agents/src/io.rs) — 原子写助手（临时文件 + fsync + rename）
- [windows_serve.rs](../../../crates/agents/src/windows_serve.rs) — Windows 侧服务边界
- [testutil.rs](../../../crates/agents/src/testutil.rs) — 池管理测试共用工具

## 相关
- [agents/core](../../foundation/core/index.md) — 引用卡与资源根类型
- [dag-runtime](../../workflow/dag-runtime/index.md)、[执行约定](../../../rules/04-dag-execution-contract.md)
