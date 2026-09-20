Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# local 模块

本地 CLI 前端：clap 命令解析 + headless 运行时。
包名 `opencoder-local`；远程管理是独立二进制 `opencoder-cli`（[ctl](../ctl/index.md)）。

## 索引
- `src/lib.rs` — clap `Cli` 与子命令
- `src/run.rs` — `run_headless` 主入口
- `src/ts/` — tmux 会话与中央注册表
- `src/todos_cmd.rs` — todos 子命令
- 仓库根 `src/main.rs` — 二进制 `opencoder` 入口
