Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# Windows 原生支持

原生 TUI 与 Operator 节点（无 WSL）；边界、安装与人工验收见 windows 文档。

## 索引

- [安装、运行、构建与人工验收](../../../docs/windows.md)
- [scripts/install.ps1](../../../scripts/install.ps1) — Windows 安装器
- [crates/core/src/platform/](../../../crates/core/src/platform) — 共享宿主操作
  （`shell.rs` 命令工具、`user_dirs.rs` 目录隔离、`windows_acl.rs`、
  `run_as.rs`）
- [windows_serve.rs](../../../crates/agents/src/windows_serve.rs)
  — Windows 节点的只读资源服务
- [crates/agent/src/main.rs](../../../crates/agent/src/main.rs) —
  `opencoder-agent --remote` Operator 节点
- [crates/tui/src/windows_console.rs](../../../crates/tui/src/windows_console.rs)
  — TUI 控制台适配
- [tests/windows_operator.rs](../../../crates/worker/tests/windows_operator.rs)
  — Windows Operator 回归

## 相关

- [共享平台逻辑](../../../agents/foundation/core/index.md)、
  [会话与进程](../../../agents/runtime/session/index.md)
- [TUI](../../../agents/frontend/tui/index.md)、
  [Operator 节点](../../../agents/execution/worker/index.md)
- [Harness](../../interaction/harness/index.md)、[调度平台](../agent-platform/index.md)
