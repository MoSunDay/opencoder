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
- [src/session_cmd.rs](../../../crates/local/src/session_cmd.rs) — `session` 子命令；
  `list/show/export` 经 `TursoStore::open_reader` 只读打开（不取维护租约、库不存在即
  视为无会话，不为了报告空结果建库；`export` 携带 orphan 输入且不修复；schema 过旧时报告记录的
  版本与补救办法，引擎错误保留为 cause），`delete/import` 仍是多写者 WAL 下的普通写者；
  `tests/session_reader_dispatch.rs`、`tests/session_reader_old_schema.rs`
- [src/todos_cmd.rs](../../../crates/local/src/todos_cmd.rs) — todos 子命令；
  `list/show/events` 只读打开（同上），run/resume 等变更子命令仍走建库写者；
  `interrupt`（[src/todos_interrupt.rs](../../../crates/local/src/todos_interrupt.rs)）以写者直接
  提交挂起（CAS `Suspended` + 一条带 `reason` 的 `workflow_interrupted` 事件），不等 driver 确认，
  只删除旧构建留下的 intent 文件；`todos_interrupt/tests.rs` 与根目录 `tests/todos_interrupt_e2e.rs`；
  `tests/todos_reader_dispatch.rs`、`tests/todos_cli_dispatch.rs`
- [src/ts/](../../../crates/local/src/ts) — tmux 会话与中央注册表；`ts -l` 与 attach-only
  resume 经 `read_records` 只读（不取租约、不同步 live workdir、不迁移）；其余 ts 命令是
  多写者 WAL 下的短命写者，只在注册/删行期间打开 `TsRegistry::open`，attach/spawn 期间不持有；
  legacy 扫描 reader 优先，schema 过旧时退回迁移写者（`TursoStore::open`），存活写者旁边照常
  迁移；`ts -c` / `ts -d` 直接写进存活会话正打开着的 store（没有 `kept_in_use` / 重试策略）；
  `ts/concurrency_tests.rs` 以子进程持有者覆盖上述并存；见
  [changelog](../../../features/changelog/2026-11-28/database-multi-writer-wal-and-maintenance-lease.md)
- [src/main.rs](../../../src/main.rs) — 仓库根二进制 `opencoder` 入口
- Windows 支持边界见 [Windows 能力卡](../../../features/platform/windows/index.md)
