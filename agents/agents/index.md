Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# agents 模块

版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。

## 索引
- `crates/agents/src/` — 池管理、引用卡、NFS 导出
- `crates/agents/src/references.rs` — memory 引用扫描

## 相关
- [agents/core](../core/index.md) — 引用卡与资源根类型
