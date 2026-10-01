Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# agents 模块

版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。

## 索引
- `crates/agents/src/` — 池管理、引用卡、NFS 导出
- `crates/agents/src/references.rs` — memory 引用扫描
- [snapshot](../../crates/agents/src/snapshot/mod.rs) — 只固定本次 Agent 所需的当前版本依赖；拒绝软链接与特殊文件，流式校验完整目录摘要，不复制无关历史版本。
- [nfs/acl.rs](../../crates/agents/src/nfs/acl.rs)、[serve/transport.rs](../../crates/agents/src/serve/transport.rs) — 只读 NFS 的 Linux ACL 查询与端口注册；ACL 写入始终拒绝，支持源工作区的内核写时复制。

## 相关
- [agents/core](../core/index.md) — 引用卡与资源根类型
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
