Commit: 4a6a4181c3cce8359e1668735b4e69695a0daf2a

# store 模块

`Store` trait + Turso（WAL）持久化层。细节以代码为准。
接缝：`Arc<dyn Store>`，session/web/todos/control 等全部经此持久化。
项目接口 `ProjectStore` 由同一 `TursoStore` 实现；独立 Web 在
[serve](../../../crates/web/src/lib.rs) 中复用同一实例。

## 索引
- [src/store.rs](../../../crates/store/src/store.rs) — `Store` trait（宏内定义，
  `lib.rs` re-export）；[contract/](../../../crates/store/src/store/contract) 与
  [impl_methods/](../../../crates/store/src/turso_store/impl_methods) 分组
  组装接口与实现
- [src/turso_store/](../../../crates/store/src/turso_store) — Turso 实现（WAL）；全部写路径经 `opencoder-database::busy`（BEGIN IMMEDIATE 有界 Busy 重试）取跨进程写锁，`tx.rs` 为其 re-export 垫片
- [src/jsonl.rs](../../../crates/store/src/jsonl.rs) — `JsonlStore`
- [src/project.rs](../../../crates/store/src/project.rs) — `ProjectStore`
  项目目录与执行持久化
- [src/session_store.rs](../../../crates/store/src/session_store.rs) —
  `SessionStore`
- [src/ts_registry.rs](../../../crates/store/src/ts_registry.rs) — tmux 会话
  中央注册表
- [src/types.rs](../../../crates/store/src/types.rs) — `SessionMeta`、
  `SessionFilter` 等 DTO
- [src/schedule_types.rs](../../../crates/store/src/schedule_types.rs)、
  [schedule.rs](../../../crates/store/src/turso_store/execution/schedule.rs) —
  调度台账与定义表
- [schema.rs](../../../crates/store/src/turso_store/bootstrap/schema.rs)
  （bootstrap/）— schema 初始化与版本升级；只读版本探针让 schema 就绪的
  暖打开跳过写锁引导+checkpoint（不等待跨进程写锁）；升级链与专项迁移在
  [bootstrap/schema/](../../../crates/store/src/turso_store/bootstrap/schema)
  （migrations.rs、catalog.rs、access.rs、project_links.rs 等）
- [tokens.rs](../../../crates/store/src/turso_store/identity/users/tokens.rs) —
  用户身份与独立 Token；公共身份面另经
  [src/users.rs](../../../crates/store/src/users.rs) 暴露
  （`AccessToken`/`PlatformUser`，lib.rs re-export）
- [sessions.rs](../../../crates/store/src/turso_store/conversation/sessions.rs)
  — 会话读写；
  [messages.rs](../../../crates/store/src/turso_store/conversation/messages.rs)
  — 消息与分页读取（`load_transcript_page`）；
  [node_tasks.rs](../../../crates/store/src/turso_store/execution/node_tasks.rs)
  — 节点任务
- [src/fleet/](../../../crates/store/src/fleet/mod.rs) — 独立 fleet 控制库：
  节点目录与执行索引；[handoff/](../../../crates/store/src/fleet/handoff/mod.rs)
  冻结请求与回执；[report/](../../../crates/store/src/fleet/report.rs) 协商报告
  （[snapshot.rs](../../../crates/store/src/fleet/report/snapshot.rs)、
  [rows.rs](../../../crates/store/src/fleet/report/rows.rs)）；
  [inventory_meta/](../../../crates/store/src/fleet/inventory_meta/mod.rs)；
  [records.rs](../../../crates/store/src/fleet/records.rs) 终态索引批删
- [inventory/](../../../crates/store/src/turso_store/execution/inventory)、
  [reporting/](../../../crates/store/src/turso_store/execution/reporting) —
  执行索引变更记录与 Brain 报告队列
  （`turso_store/execution/` 下）
- [brain_layered/](../../../crates/store/src/turso_store/execution/brain_layered)
  — 分层画布投影（含 schema.rs）
- [database](../../../crates/database/src/lib.rs) — 统一执行 Store、Fleet、
  TsRegistry、Ontology 的数据库操作；maintenance serve-bridge（
  [bridge.rs](../../../crates/database/src/maintenance/bridge.rs)）将 stdin
  空闲超 30s 的写事务回滚
- [session_index](../../../scripts/maintenance/session_index/README.md) —
  历史会话排序索引维护
- [brain_cleanup](../../../scripts/maintenance/brain_cleanup/__main__.py) —
  历史大脑表清理
- [concurrency.rs](../../../crates/database/tests/concurrency.rs)、
  [processes.rs](../../../crates/database/tests/processes.rs)、
  [storage_mixed.rs](../../../crates/store/examples/storage_mixed.rs) — 并发、
  多进程与混合负载回归

## 相关
- [event](../event/index.md)
