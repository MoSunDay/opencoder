# Turso 文件库跨进程写锁竞争根治：BEGIN 有界重试 + 暖开快路径 + 桥看门狗

一句话：本进程池内单写 lane 不可能自锁，偶发 `Busy("database is locked")`
全部来自共享同一 `opencoder.db` 的其他进程（短命 CLI 开库抢写锁跑
bootstrap/checkpoint、第二个 TUI、`ts` 级联删除、`storage serve` 桥挂起
写事务超 30s）。本次按三层防御削减锁冲突面，不改公共 API / Ontology。
第 3 层（runner 对 `record_checked` 失败的高层重试）默认不做，保持失败
语义简单，观察前两层效果。

## 行为口径

- **BEGIN 有界重试（核心）**：`run_tx` 的 `BEGIN IMMEDIATE` 失败且错误链
  为 `turso::Error::Busy / BusySnapshot`、连接仍在 autocommit（事务未开、
  无副作用）时，指数退避 + 抖动重试：25ms→1s 封顶（抖动 ≤ 基值 1/4），
  总预算 120s，每次失败 `tracing::warn` 记录次数/耗时。退避与预算决策为
  纯函数（jitter 由参数注入）。其他错误或预算耗尽立即上抛。
- **暖开快路径**：`TursoStore::open`/`open_catalog` 先在**读 lane** 探测
  `schema_version`；水位已是当前值时整个跳过 DDL bootstrap + checkpoint
  写事务（幂等 DDL 本就不需要重跑），短命 CLI 不再每次开库抢写锁。
  runtime 开库额外要求三张 runtime 专属表存在——共享 catalog 库记录同一
  水位但不装这些表，runtime 开库仍会走全量 bootstrap 补装（既有目录测试
  契约保持）。
- **checkpoint best-effort**：慢路径上 `wal_checkpoint(PASSIVE)` 遇 Busy
  记 debug 日志后吞掉（他人正在写/checkpoint 是常态，`wal_autocheckpoint`
  仍会收敛 WAL），不再让开库失败。门控输入取自**开库前**的文件存在性
  （`Database::open` 可能自行建文件，新库首开必须确定性跳过 checkpoint）。
- **桥看门狗**：`storage serve` 诊断桥持有未提交写事务时，stdin 超过 30s
  无新行自动 ROLLBACK 并继续服务；autocommit / read_only 状态不持写锁、
  无限期等待不变。`serve` 重构为通用流读写（公共签名不变），看门狗时限
  可注入以便测试。

## 不变量

- 快路径只读探测；任何读失败（无版本表/损坏）回退全量 bootstrap，由其
  决定（含拒绝更高版本的既有错误）。
- 重试仅在 autocommit 保持时进行，杜绝重复 BEGIN 叠加副作用。
- catalog 库不装 runtime 表的既有契约（子进程测试）不受快路径影响。

## 功能 → 测试

| 功能 | 测试 |
|------|------|
| 退避指数增长/封顶 | `opencoder-store` lib `turso_store::tx::tests::begin_retry_delay_grows_exponentially_then_caps` |
| 抖动 ≤ 基值 1/4 | `turso_store::tx::tests::begin_retry_delay_jitter_stays_within_a_quarter_of_base` |
| 预算耗尽即停 | `turso_store::tx::tests::begin_retry_delay_stops_once_the_budget_is_spent` |
| Busy/BusySnapshot 链识别 | `turso_store::tx::tests::busy_errors_are_detected_through_context_wrappers` |
| 跨连接写锁冲突下重试成功 | `turso_store::tx::tests::run_tx_survives_cross_connection_write_lock_contention`（受害者 busy_timeout=50ms vs 持有者 300ms，去重试化验证过必失败） |
| 暖开不抢写锁 | `opencoder-store` `schema_bootstrap::suite_2::current_schema_opens_while_an_external_writer_holds_the_lock` |
| 桥停滞自动回滚 | `opencoder-database` lib `maintenance::bridge::tests::stalled_client_has_its_open_write_transaction_rolled_back` |
| 桥短停顿不误伤 | `maintenance::bridge::tests::active_client_keeps_its_transaction_across_short_pauses` |

回归：`cargo test -p opencoder-store --test store_concurrency --test inputs_cross_instance_serialized --test schema_bootstrap`、`cargo test -p opencoder-store --lib`（含共享 catalog 子进程契约）、`cargo test -p opencoder-database`、两 crate clippy 零警告。

## 涉及文件

- `crates/store/src/turso_store/tx.rs`（重试 + 纯函数决策 + 测试）
- `crates/store/src/turso_store/bootstrap/schema/parts/part_1.rs`（`bootstrap_is_noop` 探测）
- `crates/store/src/turso_store/bootstrap/connection/mod.rs`（暖开快路径 + checkpoint best-effort）
- `crates/database/src/maintenance/bridge.rs`（通用流 + 看门狗 + 测试）
- `crates/store/tests/schema_bootstrap/suite_2.rs`（新增）、`schema_bootstrap.rs` / `suite_1.rs`（注释随快路径口径更新）
