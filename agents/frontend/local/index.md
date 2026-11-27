Commit: 6c3491fd94ec48f28aa51058feb4dd7c9e3daf83

# local 模块

本地 CLI 前端：clap 命令解析 + headless 运行时。
包名 `opencoder-local`；远程管理是独立二进制 `opencoder-cli`
（[ctl](../../platform/ctl/index.md)）。

## 索引
- [src/lib.rs](../../../crates/local/src/lib.rs) — clap `Cli` 与子命令
- [src/run.rs](../../../crates/local/src/run.rs) — `run_headless` 主入口
- [src/model_override.rs](../../../crates/local/src/model_override.rs) — headless
  进程级模型按 `--model` > `OPENCODER_MODEL` > Resume/Fork 持久化的
  Shared/Act/Plan 会话选择 > per-mode config 确定；`preferred_explicit_model`
  只保留前两者中的胜者，Resume/Fork 恢复后重应用同一胜者。显式覆盖清除
  Shared/Act/Plan 会话与配置模式 `model` 选择，保留生成参数与 context overlay；
  已落行的模式感知会话写入空元数据 `{}`，legacy 行保持 `NULL`。尚未落行的新会话
  只更新内存，首次正常持久化再写最终模型/元数据，不因启动覆盖预建行。显式/Env
  持久化错误向上传播，并补偿 `sessions.model` 与内存状态，避免磁盘旧值/内存新值分裂
- [src/daemon.rs](../../../crates/local/src/daemon.rs) — `daemon` 子命令
- [src/ts/](../../../crates/local/src/ts) — tmux 会话与中央注册表
- [src/todos_cmd.rs](../../../crates/local/src/todos_cmd.rs) — todos 子命令
- [src/main.rs](../../../src/main.rs) — 仓库根二进制 `opencoder` 入口
- Windows 支持边界见 [Windows 能力卡](../../../features/platform/windows/index.md)
