Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# local 模块

本地 CLI 前端：clap 命令解析 + headless 运行时。
包名 `opencoder-local`；远程管理是独立二进制 `opencoder-cli`
（[ctl](../../platform/ctl/index.md)）。

## 索引
- [src/lib.rs](../../../crates/local/src/lib.rs) — clap `Cli` 与子命令
- [src/run.rs](../../../crates/local/src/run.rs) — `run_headless` 主入口
- [src/daemon.rs](../../../crates/local/src/daemon.rs) — `daemon` 子命令
- [src/session_cmd.rs](../../../crates/local/src/session_cmd.rs) — session 子命令；`open_store`/`open_store_at` 是 CLI 与 TUI 共享的会话库启动序列（建目录 → 直接打开 `opencoder.db.rdb`）
- [src/ts/](../../../crates/local/src/ts) — tmux 会话与中央注册表
- [src/todos_cmd.rs](../../../crates/local/src/todos_cmd.rs) — todos 子命令
- [src/main.rs](../../../src/main.rs) — 仓库根二进制 `opencoder` 入口
- Windows 支持边界见 [安装与构建](../../../docs/windows.md)
