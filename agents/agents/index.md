Commit: 1374d7ed231300bd2d00e13790c6f94da3ac9812

# agents 模块

版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。

## 索引
- `crates/agents/src/` — 池管理、引用卡、NFS 导出
- `crates/agents/src/references.rs` — memory 引用扫描
- [snapshot](../../crates/agents/src/snapshot/mod.rs) — 固定资源当前版本，支持全池或指定 Agent 的依赖范围；拒绝软链接与特殊文件，流式校验完整目录摘要，不复制无关历史版本。私有暂存树逐文件落盘，填充后的目录自底向上同步，共享类别目录在所有复制任务结束后同步，最后原子发布；已发布快照重试时保持不变。回归见 [snapshot/tests.rs](../../crates/agents/src/snapshot/tests.rs)。
- [nfs/acl.rs](../../crates/agents/src/nfs/acl.rs)、[serve/transport.rs](../../crates/agents/src/serve/transport.rs) — 只读 NFS 的 Linux ACL 查询与端口注册；ACL 写入始终拒绝，支持源工作区的内核写时复制。
- [testutil.rs](../../crates/agents/src/testutil.rs) — 资源追加、引用扫描及池管理测试共用全局资源目录锁，持锁期间使用独立临时目录。

## 相关
- [agents/core](../core/index.md) — 引用卡与资源根类型
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
