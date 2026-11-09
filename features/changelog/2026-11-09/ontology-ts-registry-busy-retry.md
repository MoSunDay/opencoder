# Ontology 与 ts registry 写路径接入跨进程写锁有界重试

一句话：`opencoder-ontology` 的全部写事务入口与 `opencoder-store` 的
`ts_registry` 写路径改为经共享 `opencoder-database::busy` 助手获取
`BEGIN IMMEDIATE`——错误链为 `Busy/BusySnapshot` 且连接仍在 autocommit
时按 25ms→1s 指数退避（总预算 120s）重试，跨进程写租约不再让写请求
瞬时失败。

## 行为口径

- `mutations.rs` HTTP 中间件事务、`catalog.rs` / `freshness.rs` 数据面
  事务、`schema::bootstrap` DDL 事务：仅 BEGIN 换成
  `busy::begin_with_retry`，COMMIT/ROLLBACK 语义与事务边界不变
  （事务有意横跨 handler，不做收窄）。
- Store 写方法（`put` / 环境写入 / 向量写入等）运行在请求作用域
  `REQUEST_CONNECTION` 上，由外层事务保护，**不重复包裹**（避免嵌套
  BEGIN）。全仓核查：非测试构建里 `Database::store()` 无
  `REQUEST_CONNECTION` 即 panic，作用域只存在于上述三处入口与
  schema bootstrap，ctl 不依赖 ontology，control 仅经 AppState 入口
  触达写路径——不存在独立直达的裸写调用点。
- `ts_registry` 的 `open` DDL 批、`upsert`、`delete`、`mark_migrated`：
  各自用 `busy::run_tx` 包裹单条语句批，SQL/参数/返回值与错误上下文
  原样保留。

## 功能 → 测试

| 功能 | 测试 |
|------|------|
| 外进程持写租约时 catalog apply 经重试落库 | `opencoder-ontology` `tests/catalog.rs::catalog_apply_queues_past_a_foreign_write_lease`（子进程持 BEGIN IMMEDIATE 31s > 池连接 30s busy 预算；去重试化验证：裸 BEGIN 30s 后 `Busy("database is locked")` 必失败，重试路径 ~32s 通过） |
| ts registry 四个写路径语义不变 | `opencoder-store` lib `ts_registry::tests::{upsert_get_list_roundtrip,upsert_is_idempotent_and_replaces,delete_removes_only_the_target,meta_marker_roundtrip,on_disk_open_is_reopenable}` |

回归：`cargo test -p opencoder-ontology`（66 例）、
`cargo test -p opencoder-store ts_registry`（5 例）、
`cargo clippy -p opencoder-ontology -p opencoder-store --all-targets`
（本次改动零新增警告）。

## 涉及文件

- `crates/ontology/src/mutations.rs` / `catalog.rs` / `freshness.rs` /
  `database/schema/mod.rs`（BEGIN 换 `begin_with_retry`）
- `crates/store/src/ts_registry.rs`（四处 `run_tx` 包裹）
- `crates/ontology/tests/catalog.rs`（新增跨进程竞争测试）
