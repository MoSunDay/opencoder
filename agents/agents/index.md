Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# agents 模块

版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。

## 索引
- `crates/agents/src/` — 池管理、引用卡、导出；写侧 memory 任意安全相对路径（多文件/子目录），`references::scan_memory` 递归含 ≥1 个 `*.md` 即命中
- 引用卡字段 `run_mode`（`opencoder_core::agent::RunMode`：`Operator` 默认 host 进程会话运行时 / `Agent` 每回合只读 runc 沙箱；serde 缺省容错旧卡）：`create_agent_with_profile` 写入新卡，`update_agent_with_profile` 变更时追加 `run_mode` history 条目、`None` 不动（镜像 harness 处理）

## 相关
- [agents/core](../core/index.md) — 引用卡与资源根类型
