Commit: 0abfe0c77

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
- [src/store/errors.rs](../../../crates/store/src/store/errors.rs) — 存储错误分类：
  FK 违例 / `SQLITE_CORRUPT` / worker panic；session 持久化重建与列表坏行降级共用，
  crate 根导出 `is_foreign_key_violation`、`is_storage_corruption`
- [src/turso_store/](../../../crates/store/src/turso_store) — Turso 实现（WAL）；`open_reader` 只读打开（巡检命令用，不取写者锁、不 bootstrap/迁移）；写者池打开即挂载周期 PASSIVE checkpoint（仅写者锁持有者执行）（`opencoder-database/src/checkpoint.rs`：60s/1MiB 门限、独立诊断连接、busy 即跳过；PASSIVE 只回填已提交帧不截断文件，TRUNCATE 仍 CLI-only）；全部写路径经 `opencoder-database::busy`（BEGIN IMMEDIATE 有界 Busy 重试）取跨进程写锁，`tx.rs` 为其 re-export 垫片
- [src/jsonl.rs](../../../crates/store/src/jsonl.rs) — `JsonlStore`
- [src/project.rs](../../../crates/store/src/project.rs) — `ProjectStore`
  项目目录与执行持久化
- [src/session_store.rs](../../../crates/store/src/session_store.rs) —
  `SessionStore`
- [src/ts_registry.rs](../../../crates/store/src/ts_registry.rs) — tmux 会话
  中央注册表；`TsRegistry::open` 是多写者 WAL 下的普通写者（池持共享维护租约，事务经
  `BEGIN IMMEDIATE` 有界重试串行化），`open_reader` 只读（TUI 镜像常驻持有）；ts 命令与 TUI 镜像
  都只在一次变更期间打开写者，无人长期持有 `ts.db` 写者
- [src/bundle.rs](../../../crates/store/src/bundle.rs) — 会话二进制 bundle 导出/导入；导出只读：
  输入 = pending 行 + `Store::orphan_inputs`（已 promote 未 record 的行以 pending 形态导出），不在
  导出路径就地修复，reader 与写者对同一行导出相同结果
- [src/types.rs](../../../crates/store/src/types.rs) — `SessionMeta`、
  `SessionFilter` 等 DTO；模式模型覆盖是独立元数据，不并入 `SessionMeta`
- [src/session_models.rs](../../../crates/store/src/session_models.rs) —
  `SessionModelOverrides { shared, act, plan }`；schema v42 增加可空
  `sessions.model_overrides_json`，整体 `NULL` 表示 legacy 单模型行为，`{}`
  （含缺省字段）表示模式感知且未设置会话选择，字段有值时覆盖对应 per-mode
  config；其中 Shared 值立即优先于 `modes.shared.model`。`SessionState::new`、Web
  等正常新建入口显式写入 `Some(default())`，底层 `Store::create_session` 仍保留
  legacy `NULL` 默认；历史行不被回填，旧 bundle 缺少该字段时反序列化为
  `None`，导入保留其 legacy `NULL`，已有 bundle 元数据也原样传递。显式模型只把
  已有模式感知行的 Shared/Act/Plan 清成 `{}`，legacy 行保持 `NULL`；Env reload
  写入目标 Env 的 Shared 基值并以 `{}` 清空全部临时会话选择。Store 经
  `create_session_with_model_overrides`、`get_session_model_overrides`、
  `set_session_model_overrides` 访问
- [src/schedule_types.rs](../../../crates/store/src/schedule_types.rs)、
  [schedule.rs](../../../crates/store/src/turso_store/execution/schedule.rs) —
  调度台账与定义表
- [schema.rs](../../../crates/store/src/turso_store/bootstrap/schema.rs)
  （bootstrap/）— schema 初始化与版本升级；只读版本探针让 schema 就绪的
  暖打开跳过写锁引导+checkpoint（不等待跨进程写锁），探针同时校验活跃索引
  与 guidance 队列形状，legacy 形状仍走完整 bootstrap；升级链与专项迁移在
  [bootstrap/schema/](../../../crates/store/src/turso_store/bootstrap/schema)
  （migrations.rs、catalog.rs、access.rs、project_links.rs、session_activity.rs 等；v41 活跃索引换形与坏页探测/回填，v42 增加可空会话模式模型覆盖元数据）
- [tokens.rs](../../../crates/store/src/turso_store/identity/users/tokens.rs) —
  用户身份与独立 Token；公共身份面另经
  [src/users.rs](../../../crates/store/src/users.rs) 暴露
  （`AccessToken`/`PlatformUser`，lib.rs re-export）
- [sessions.rs](../../../crates/store/src/turso_store/conversation/sessions.rs)
  — 会话读写；活跃索引维护
  [sessions/activity.rs](../../../crates/store/src/turso_store/conversation/sessions/activity.rs)
  （`IdxDelete` 重建并重试一次，重建先建探针索引再换名；重建撞上不可读存储时跳过活跃时间戳降级写入）、列表分页/坏行降级
  [sessions/listing.rs](../../../crates/store/src/turso_store/conversation/sessions/listing.rs)
  （`updated_at` 活跃时间、点查 fallback 跳过坏行）；行解码对
  `id` / `created_at` / `updated_at` 保持严格（不默认、不跳行），失败带
  `decode sessions.<column> for session <id>` 定位；
  [messages.rs](../../../crates/store/src/turso_store/conversation/messages.rs)
  — 消息与分页读取（`load_transcript_page`）；行解码对 legacy 可空
  `usage_json` / `synthetic` 做 NULL 兼容（默认 usage / `false`），其余列严格并带
  `decode messages.<column> for session <id> message <id>` 定位；
  [events.rs](../../../crates/store/src/turso_store/conversation/events.rs)
  — 事件追加与有界读取（`events_page` 翻页、`events_tail` 最新窗口 ≤max_rows+payload 预算，web SSE 首连回放在用）；
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
  TsRegistry、Ontology 的数据库操作；turso `0.9.0-pre.2` 多进程 WAL（`<db>-wal` + 协调文件
  `<db>-tshm`），任意进程都是写者，事务由引擎 WAL 写锁 + `busy::run_tx` 的 `BEGIN IMMEDIATE`
  有界重试串行化；`Intent::{Writer, Reader}` 显式打开意图：写者（`Database::open`，默认）经
  [engine.rs](../../../crates/database/src/engine.rs) `open_writer` 打开——`<db>-tshm` 缺失即首次
  打开，在 `<db>.maintenance.lock` 的**独占**租约下初始化后降级为**共享**租约并持有到池释放，
  其他在同一时刻打开的写者对共享租约等待（`MaintenanceActive`，120s 预算），协调文件仍在初始化
  的打开在 10s 内重试（[maintenance_lock.rs](../../../crates/database/src/maintenance_lock.rs)：
  `acquire_shared` / `try_acquire_exclusive`、`WritersActive` = "writers still active"）；
  reader（`Database::open_reader` / `open_with(.., Intent::Reader)`，store 侧为 `TursoStore::open_reader`）
  不取租约、引擎只读、连接 `query_only`、不建 marker、不注册也不跑 checkpoint，`write` 被拒；
  checkpoint 句柄经 `Weak` 绑定到发布它的池，周期循环（`periodic_pass`）每轮剔除池已释放的失效
  句柄，每个写者进程各跑 PASSIVE、引擎 checkpointer 锁仲裁；`run_once` 复用存活池的句柄或在
  自己的共享租约下开私有引擎；TRUNCATE 与可写 bridge 取**独占**租约，写者存活时快速失败
  `WritersActive`；引擎从不删除 `-wal`/`-tshm`，不要在进程打开着库时手工删除它们；
  maintenance serve-bridge（
  [bridge.rs](../../../crates/database/src/maintenance/bridge.rs)）将 stdin
  空闲超 30s 的写事务回滚，只在非只读时取独占租约（见
  [changelog](../../../features/changelog/2026-11-28/database-multi-writer-wal-and-maintenance-lease.md)）
- [session_index](../../../scripts/maintenance/session_index/README.md) —
  历史会话排序索引维护
- [brain_cleanup](../../../scripts/maintenance/brain_cleanup/__main__.py) —
  历史大脑表清理
- [db_repair](../../../scripts/maintenance/db_repair/README.md) — Turso 库损坏
  （`IdxDelete`/`FOREIGN KEY`/坏页）时 `.dump` + 页级 carve 重建新库
- [concurrency.rs](../../../crates/database/tests/concurrency.rs)、
  [processes.rs](../../../crates/database/tests/processes.rs)（多写者子进程 + reader）、
  [maintenance_lock.rs](../../../crates/database/tests/maintenance_lock.rs)（子进程写者：共享/独占
  租约互斥、`WritersActive` / `MaintenanceActive`）、
  [first_open_race.rs](../../../crates/database/tests/first_open_race.rs)（多子进程同时首次打开）、
  [busy_contention.rs](../../../crates/database/tests/busy_contention.rs)（`run_tx` 有界重试）、
  [checkpoint_binding.rs](../../../crates/database/tests/checkpoint_binding.rs) /
  [checkpoint.rs](../../../crates/database/tests/checkpoint.rs)（句柄绑定池、多池、缺库/`:memory:` 拒绝）、
  [multi_writer_soak.rs](../../../crates/database/tests/multi_writer_soak.rs)（`#[ignore]` 多写者压测：
  杀掉一个写者后其余推进、独占租约可得、TRUNCATE 后 WAL 为空）、
  [database_paths.rs](../../../crates/database/tests/database_paths.rs)（符号链接路径统一）、
  [maintenance.rs](../../../crates/database/tests/maintenance.rs)（TRUNCATE 独占租约规则）、
  [schema_bootstrap_race.rs](../../../crates/store/tests/schema_bootstrap_race.rs)（多子进程同时对
  全新文件 bootstrap），
  [reader_open.rs](../../../crates/store/tests/reader_open.rs)（store reader 与存活写者并存）、
  [store_integration/bundle.rs](../../../crates/store/tests/store_integration/bundle.rs)
  （reader 导出携带 orphan 输入）、
  [storage_mixed.rs](../../../crates/store/examples/storage_mixed.rs) — 并发、
  多进程与混合负载回归

## 相关
- [event](../event/index.md)
