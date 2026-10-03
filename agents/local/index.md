Commit: 40dd45ed4c7c3240a0e879ac5bfec391ffb5a03c

# local 模块

本地 CLI 前端：clap 命令解析 + headless 运行时。
包名 `opencoder-local`；远程管理是独立二进制 `opencoder-cli`（[ctl](../ctl/index.md)）。

## 索引
- `src/lib.rs` — clap `Cli` 与子命令
- `src/run.rs` — `run_headless` 主入口
- `src/daemon.rs` — `daemon --server/--client` 迁移提示：指向 `opencoder-server`/`opencoder-agent`；凭据一律不回显，显式 token 时提示 `--token-file`
- `src/ts/` — tmux 会话与中央注册表
- `src/todos_cmd.rs` — todos 子命令
- 仓库根 `src/main.rs` — 二进制 `opencoder` 入口

- Windows 使用原生 TUI 与 PowerShell 7.4 及以上的 7.x 稳定版；`ts` 拒绝 tmux 模式。[安装与构建](../../docs/windows.md) 使用 Windows ZIP 与 PowerShell 安装器。
