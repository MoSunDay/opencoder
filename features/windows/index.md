Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# Windows 原生支持

Windows 11 x64 可原生运行 OpenCoder TUI 与 Operator 节点，无需 WSL，需要 PowerShell 7.4 及以上的 7.x 稳定版。

## 支持边界

- TUI 支持本地任务、恢复、模型切换、图片输入和原生 `--wrap codex`。图片快捷键可配置；终端占用默认组合键时，可按 [使用说明](../../docs/windows.md) 改为 `Ctrl+Alt+V`。
- Windows 命令工具为 `powershell`，Linux/macOS 为 `bash`。只读模式保守检查 PowerShell 语法，拒绝动态调用、脚本块、重定向及未确认只读的命令。
- Windows 节点只接受 Operator，连接已有 Server；DAG/runc、Brain、Team、Todos、Project、Maintenance、Windows Server 部署、服务与滚动发布不在范围内。
- Operator 有独立工作目录与配置快照，Windows 用户目录变量随任务隔离。配置与凭据文件采用受保护的 ACL。
- 项目只填写部分 Codex 配置时，保留全局未填写的启动程序、凭据目录和环境变量；指定同名 profile 时替换该 profile 的完整版本，其他 profile 保留。`null` 可清除 Codex 配置或可选字段；最终私有配置超过容量限制时拒绝加载，错误不显示配置值。
- 取消、超时、节点退出清理该次执行的整个进程树；命令支持后台运行、查看和停止。终端退出时恢复控制台模式。
- ZIP 包包含 TUI 与节点程序、版本信息及校验和。安装器按整组程序升级，要求先退出运行进程；失败恢复旧目录，成功保留上一组程序。

## 使用与实现

- [安装、运行、构建与人工验收](../../docs/windows.md)
- [共享平台逻辑](../../agents/core/index.md)、[会话与进程](../../agents/session/index.md)
- [TUI](../../agents/tui/index.md)、[Operator 节点](../../agents/worker/index.md)
- [Harness](../harness/index.md)、[调度平台](../agent-platform/index.md)
