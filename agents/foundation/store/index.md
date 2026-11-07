# store 模块

`Store` trait + RocksDB（rdb）持久化层。细节以代码为准。
接缝：`Arc<dyn Store>`，session/web/todos/control 等全部经此持久化。
项目接口 `ProjectStore`、`FleetStore`、`TsRegistry` 均由 rdb 实现；
Ontology 保留独立 SQLite 后端（`opencoder-database`）。

## 索引
- [src/store.rs](../../../crates/store/src/store.rs) — `Store` trait（宏内定义，
  `lib.rs` re-export）；[contract/](../../../crates/store/src/store/contract) 与
  [impl_methods/](../../../crates/store/src/rdb_store/impl_methods) 分组组装接口与实现
- [src/rdb_store/](../../../crates/store/src/rdb_store) — RocksDB 主后端：
  [engine.rs](../../../crates/store/src/rdb_store/engine.rs) 单句柄/写租约 +
  跨进程 `<db-dir>.owner.lock` exclusive lock（busy error 含数据库/lock
  路径与持有者 pid 等非敏感元数据；不支持多进程并发写同一目录）、
  [kv.rs](../../../crates/store/src/rdb_store/kv.rs) key 家族与 JSON codec、
  [tx.rs](../../../crates/store/src/rdb_store/tx.rs) 原子批写
- [core data_dir](../../../crates/core/src/data_dir.rs) — 统一数据目录与
  `.rdb` 目录解析策略（`session_store_rdb_dir` / `runtime_store_rdb_dir`）：
  `opencoder.db.rdb` / `runtime.db.rdb` 优先，仅旧 stem 已是目录且 `.rdb`
  不存在时回退旧布局，旧 SQLite 文件绝不作为 RocksDB 目录打开
- [src/event_ledger.rs](../../../crates/store/src/event_ledger.rs) — 任意 `Store`
  到 `EventLedger` 的适配器；平台事件/监听器/暂停等待/全局 gate/消费者游标
  （`evc/<consumer-id>`）的 rdb 落在
  [rdb_store/events.rs](../../../crates/store/src/rdb_store/events.rs)
- [src/jsonl.rs](../../../crates/store/src/jsonl.rs) — `JsonlStore`
- [src/project.rs](../../../crates/store/src/project.rs) — `ProjectStore`
  项目目录与执行持久化（rdb）
- [src/session_store.rs](../../../crates/store/src/session_store.rs) — `SessionStore`
- [src/ts_registry.rs](../../../crates/store/src/ts_registry.rs) — tmux 会话中央注册表
- [src/types.rs](../../../crates/store/src/types.rs) — `SessionMeta`、
  `SessionFilter` 等 DTO
- [src/schedule_types.rs](../../../crates/store/src/schedule_types.rs) — 调度台账
- [src/rdb_store/conversation/](../../../crates/store/src/rdb_store/conversation) —
  会话、消息、事件与输入；[identity/](../../../crates/store/src/rdb_store/identity)
  — 用户与 Token
- [src/rdb_store/projects/](../../../crates/store/src/rdb_store/projects) — 项目目录、
  TODO/专项、执行关联与进度
- [src/rdb_store/execution/](../../../crates/store/src/rdb_store/execution) — 节点任务、
  DAG、schedule、inventory/reporting 与 Brain 分层画布
- [src/fleet/](../../../crates/store/src/fleet/mod.rs) — 独立 fleet 控制库：
  节点目录、执行索引、handoff 冻结请求与回执、协商报告、inventory 元数据
- [database](../../../crates/database/src/lib.rs) — Ontology/ctl 共用的 SQLite 基建
- [session_index](../../../scripts/maintenance/session_index/README.md) —
  历史会话排序索引维护
