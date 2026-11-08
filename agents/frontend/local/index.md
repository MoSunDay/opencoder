Commit: 573bb2e48fa20832a47b7bec8adc8ee3c08e8e78

# local 模块

本地 CLI 前端：clap 命令解析 + headless 运行时。
包名 `opencoder-local`；远程管理是独立二进制 `opencoder-cli`
（[ctl](../../platform/ctl/index.md)）。

## 索引
- [src/lib.rs](../../../crates/local/src/lib.rs) — clap `Cli` 与子命令
- [src/run.rs](../../../crates/local/src/run.rs) — `run_headless` 主入口
- [src/daemon.rs](../../../crates/local/src/daemon.rs) — `daemon` 子命令
- [src/ts/](../../../crates/local/src/ts) — tmux 会话与中央注册表
- [src/todos_cmd.rs](../../../crates/local/src/todos_cmd.rs) — todos 子命令
- [src/main.rs](../../../src/main.rs) — 仓库根二进制 `opencoder` 入口
- Windows 支持边界见 [Windows 能力卡](../../../features/platform/windows/index.md)
