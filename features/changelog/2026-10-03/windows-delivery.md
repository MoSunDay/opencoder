Commit: 40dd45ed4c7c3240a0e879ac5bfec391ffb5a03c

# Windows 11 TUI 与 Operator 交付验收

Windows 11 x64 的正式 MSVC 包完成原生验收，需要 PowerShell 7.4 及以上的 7.x 稳定版。Linux/macOS 保留原有宿主命令行为。

- 发布程序对应干净提交 `40dd45ed`，两程序的构建身份一致，采用静态 CRT。ZIP SHA256 为 `6cfb295ab9f9c8de316e13fb9b7eb378cd1e457b158df2a555d3d5415ab4a26f`。
- [Windows 与 macOS CI](https://github.com/MoSunDay/opencoder/actions/runs/37101220426) 均成功；Linux 全量回归 441 个目标、5,610 passed、0 failed、7 个既有 ignored，Clippy、构建、格式与 14 项全站 UI 检查通过。
- Windows 原生 core/session/TUI/worker 单元测试分别通过 307/441/1727/71 个；数据库生命周期覆盖 5,000 次连接开关与最后持有者释放文件。
- 正式 EXE 的安装、升级、恢复、回滚、锁定、并发、PATH 恢复及五处实际进程中断验收通过。六项真实模型/Codex 检查覆盖流式输出、续会话、中文引号与图片。
- TUI 10 项检查通过，覆盖首次配置、中文回复、模型菜单、缩放、剪贴板图片以及正常退出、CTRL_BREAK 退出后的控制台恢复。Windows Terminal 会占用默认 Ctrl+V；已有 `keymap.paste_image` 配置可改为 Ctrl+Alt+V，实机按键与真实 Codex 图片识别已验证。
- Operator 24 项检查通过，覆盖 Linux Server 注册、准入与目录隔离、真实模型、幂等输入、取消、Server 重连、节点退出清理与显式恢复。固定程序和配置完成 900 秒观察，31 次检查均通过；设备恢复与维护释放已核验。

## 代码与测试入口

- [安装与使用](../../../docs/windows.md)、[支持边界](../../windows/index.md)。
- [原生会话与进程测试](../../../crates/session/tests/windows_native.rs)、[Operator 测试](../../../crates/worker/tests/windows_operator.rs)、[TUI 测试](../../../tests/windows_tui.rs)。
- [数据库生命周期](../../../crates/store/tests/connection_lifecycle.rs)、[安装恢复测试](../../../scripts/platform/windows/installer-state-tests.ps1)。
- Codex 重连与失败语义见 [session 索引](../../../agents/session/index.md)；数据库依赖固定与所有权见 [store 索引](../../../agents/store/index.md)。

正式包及独立使用说明保存在 `dist/windows11/40dd45ed/`，验收回执与截图保存在同目录的 `verification/`。文档补充不改变已验收程序及其构建身份。
