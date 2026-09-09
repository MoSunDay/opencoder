Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# local 模块

本地 CLI 前端：clap 命令解析 + headless 运行时。

## 关键路径
- `src/lib.rs` — clap `Cli`：全局 flag + 子命令 run/tui/ts/daemon/config/models/session/todos/install-tools/update
- `src/run.rs` — `run_headless` headless 主入口；`pick_resume_id` resume 选择；Ctrl-C 130
- `src/session_cmd.rs` — session list/show/delete/export/import；`build_session_json` 深度观测面
- `src/agent_override.rs` — `apply_agent_override`/`reapply_resume_agent` 折入 --agent
- `src/model_override.rs`、`src/run_image.rs` — --model / --image 覆盖
- `src/daemon.rs` — `daemon` 子命令只打印 `migration_hint` 指向 opencoder-server/agent 并退出 0
- `src/todos_cmd.rs` — todos 子命令（validate/run/resume/show/events/list/interrupt）
- `src/ts/` — tmux 会话：`actions.rs` 逐参数转发新 TUI 进程；`registry.rs` 中央注册表 `<data_root>/ts.db`
- `src/main.rs`（仓库根）— 二进制 `opencoder` 入口：supervisor 分支先于 clap 解析，Linux `configure_supervisor_binary`
- `src/update.rs`、`src/install_tools.rs`、`src/exit_tips.rs`、`src/display.rs` — 自更新/工具安装/退出提示/事件渲染

## 边界
- 包名 `opencoder-local`；`opencoder --cli` 是无行为兼容标记；远程管理客户端是独立二进制 `opencoder-cli`（见 ctl）。
- `--cmd` 与位置 prompt 互斥；`ts` 别名 `rs`，裸 `ts` 总是新建会话。
- headless `run` 不暴露 steer/queue 两段式 delivery（那是 web `POST /prompt` 的 `delivery` 字段）。
- 不做终端渲染与 HTTP 服务；TUI 在 opencoder-tui，web 在 opencoder-web。

## 相关
- [agents/session](../session/index.md) — headless run/resume/fork 核心
- [agents/store](../store/index.md) — session 子命令 + bundle 导出导入
- [agents/tui](../tui/index.md) — tui/ts 子命令的界面
- [agents/todos](../todos/index.md) — todos 子命令运行时
- [Agent Harness](../../features/harness/index.md) — --wrap/--envs 使用规则
