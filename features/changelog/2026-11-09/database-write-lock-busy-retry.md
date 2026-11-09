# 全库写路径接入跨进程写锁有界重试（共享 busy 模块）

一句话：把 `turso_store/tx.rs` 的 `BEGIN IMMEDIATE` 有界 Busy 重试下沉为
共享模块 `opencoder-database::busy`，新增 `immediate_with_retry`（turso
`Transaction` 入口）与 `begin_with_retry`（自管 COMMIT/ROLLBACK 入口），
并让 store（turso_store 全部裸写、fleet 全部写路径）、ontology、ts_registry
的每一条写路径都经它取锁——跨进程写租约冲突从「瞬时 Busy 失败」变为
「25ms→1s 指数退避排队（总预算 120s）」。此为 libsql 时代
（serialize + BEGIN IMMEDIATE + run_tx）锁治理口径在 turso 引擎上的补全。

## 行为口径

- **共享模块** `crates/database/src/busy.rs`：`run_tx` / `begin_with_retry`
  从 `turso_store/tx.rs` 平移（调度纯函数与语义不变：仅当错误链为
  `Busy/BusySnapshot` 且连接仍在 autocommit 时重试，保证每次重试无副作用）；
  新增 `immediate_with_retry`——`Transaction::new_unchecked` 失败时不构造
  包装、连接留在 autocommit，重试同样安全。`turso_store/tx.rs` 变为 5 行
  re-export 垫片，`super::tx::run_tx` 等既有引用路径全部不变。
- **turso_store 裸写收口**（原先单条 autocommit 语句遇跨进程写锁直接
  Busy 报错）：identity 用户/令牌（含嵌套调用拆出的 `create_in_tx`）、
  sessions create/update/delete/clear_others/delete_many/set_harness_runtime、
  subagent_tasks、project_links、schedule 定义与运行、team_runs、reporting
  format、nodes register/delete、brain delete/upsert_vector——各自包进
  `run_tx(BEGIN IMMEDIATE)`，SQL/参数/返回值/错误上下文原样保留；已处于
  外层事务内的调用点（bootstrap、brain 状态事务、queue in-tx 写）不重复包裹。
- **fleet 收口**：裸 `BEGIN IMMEDIATE`（report/brain/schema/records）换
  `begin_with_retry`；`Transaction::new_unchecked(.., Immediate)`（runtimes/
  capacity/receipts/acceptance/inventory/inventory_meta）换 `immediate_with_retry`
  （返回同一 `turso::Transaction`，调用点零改动）；其余单条写
  （records/runtimes/capacity/receipts/handoff DDL）包 `run_tx`。
- **池与旁路不变**：`Database` 写道信号量继续负责进程内单写者序列化；
  maintenance checkpoint 的 Busy 仍按约定显式上报；bridge 由既有 30s 空闲
  watchdog 约束，不在本次范围。

## 功能 → 测试

| 功能 | 测试 |
|------|------|
| 重试调度纯函数与 Busy 识别 | `opencoder-database` lib `busy::tests::{begin_retry_delay_grows_exponentially_then_caps, begin_retry_delay_jitter_stays_within_a_quarter_of_base, begin_retry_delay_stops_once_the_budget_is_spent, busy_errors_are_detected_through_context_wrappers}` |
| run_tx 跨连接写锁竞争下提交 | `opencoder-database` `tests/busy_contention.rs::run_tx_survives_cross_connection_write_lock_contention` |
| immediate_with_retry 排队与非 Busy 立即失败 | `opencoder-database` `tests/busy_contention.rs::{immediate_with_retry_requeues_past_write_lock_contention, immediate_with_retry_surfaces_non_busy_failures}` |
| turso_store 裸写在持锁下落库 | `opencoder-store` `tests/write_lock_contention.rs::{session_create_queues_past_foreign_write_lease, token_create_queues_past_foreign_write_lease, schedule_upsert_queues_past_foreign_write_lease}` |
| fleet 写在 31s 持锁下排队提交（去重试化验证过） | `opencoder-store` `tests/fleet_write_lock_contention.rs`（lib 31 项 fleet 测试同过） |
| 既有并发/生命周期回归 | `opencoder-store` `--lib`（75 过）、`tests/{concurrent_serialized,connection_lifecycle,inputs_cross_instance_serialized,store_concurrency}.rs`、`opencoder-ontology` 全量（66 过） |

## 实测记录

- 池连接 `busy_timeout=30s` 只吸收短于 30s 的跨进程租约；外进程持
  `BEGIN IMMEDIATE` 31s 时，裸 BEGIN 约 30s 后 `Busy("database is locked")`
  必败，重试路径 ~32s 提交成功（ontology/fleet 两条竞争测试均按此
  去重试化验证）。
