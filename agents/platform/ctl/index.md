Commit: 4a6a4181c3cce8359e1668735b4e69695a0daf2a

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端，输出 JSON。

## 索引

- `src/lib.rs` — clap 命令分发；`Storage(maintenance::StorageAction)`
  在 HTTP 装配前本地处理
- `src/http.rs` — `RequestPlan` + Bearer + 退出码
- [connection.rs](../../../crates/ctl/src/connection.rs)、
  [ctx.rs](../../../crates/ctl/src/ctx.rs) — 连接默认值与配置解析（文件/环境/参数）
- [maintenance/](../../../crates/ctl/src/maintenance/mod.rs) — Storage 子命令组
  （Serve/Query/Backup/Checkpoint）：检查/备份本机内嵌 Turso 数据库
  （`engine: turso`），本地执行不走 HTTP，底层
  `opencoder_database::maintenance`；Serve 桥在 stdin 上回滚空闲超过
  30s 的写事务（maintenance/bridge.rs `IDLE_WRITE_TX_LIMIT`）
- `src/cmd/` — 按域子命令（纯 plan() 映射）
- `src/cmd/ontology/` — Ontology 数据操作、检查与补同步；
  见 [Ontology CLI](../../../rules/09-ontology-capabilities.md)
- [cmd/brain.rs](../../../crates/ctl/src/cmd/brain.rs) — brain 子命令，与 Web 共用 API
- [cmd/project.rs](../../../crates/ctl/src/cmd/project.rs)、`src/cmd/project/`) — 项目、
  专项与 TODO API 映射（comments / context / summary / files）
- [cmd/dag.rs](../../../crates/ctl/src/cmd/dag.rs)、
  [cmd/executions.rs](../../../crates/ctl/src/cmd/executions.rs) — DAG 与执行查询
- [cmd/mod.rs](../../../crates/ctl/src/cmd/mod.rs) — HTTP 结果到退出码的映射
- `tests/` — 子命令→RequestPlan 契约与集成 e2e
- 代码：[project 命令](../../../crates/ctl/src/cmd/project.rs)、[任务文件](../../../crates/control/src/project_files)、
  [概述 API](../../../crates/web/src/api_project/summary.rs)

## 相关

- [brain](../../orchestration/brain/index.md)、[运行协议](../../../crates/control/src/api/brain_runs)
