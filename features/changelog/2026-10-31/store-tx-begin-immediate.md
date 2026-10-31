# store 事务统一 BEGIN IMMEDIATE：跨进程写者排队

一句话：把 schema 版本写入与消息追加/批量导入的最后三处 `BEGIN` 升级为 `BEGIN IMMEDIATE`，与 events/inputs 既有约定对齐——事务开始即取写锁、排队等待跨进程写者，而不是在首个 INSERT 升锁时撞上 busy_timeout 不重试的失败。

## 变更清单

- `crates/store/src/turso_store/bootstrap/schema/parts/part_2.rs:228` `set_version`：DELETE+INSERT 版本行的事务改为立即取锁，注释说明与 bootstrap 事务一致、排队等待跨进程写者。
- `crates/store/src/turso_store/conversation/messages/parts/part_1.rs:44` `append_chunk_in_tx`：消息分片追加事务立即取锁（与 input admission 同理）。
- `crates/store/src/turso_store/conversation/messages/parts/part_2.rs:21` `import_chunk_in_tx`：批量导入与 `append_chunk_in_tx` 对齐。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 追加分片/边界/空批在 IMMEDIATE 事务下行为不变 | `append_many_chunks_large_batch` / `append_many_exact_chunk_boundary` / `append_many_empty_returns_empty` | `crates/store/tests/store_concurrency.rs` |
| 批量导入分片在 IMMEDIATE 事务下行为不变 | `import_chunks_large_batch` | `crates/store/tests/store_concurrency.rs` |
| 并发写者无死锁、busy_timeout=30000、文件库 WAL 保持 | `concurrent_writers_no_deadlock` / `pragma_busy_timeout_is_30000` / `file_database_uses_wal` | `crates/store/tests/store_concurrency.rs` |

## 定向回归

- `cargo clippy -p opencoder-store --all-targets` → 0 警告。
- `cargo test -p opencoder-store --test store_concurrency` → 7 通过 / 0 失败。
- 未运行全量测试（规则 02）。

## 配置与依赖

- 无新增依赖；仅 turso_store 内部事务开启时机变化，无公共 API 变化，Ontology 目录无需同步。

## 回滚步骤

- 还原三个文件中的 `BEGIN IMMEDIATE` 为 `BEGIN` 即回滚；无数据迁移、无磁盘格式变化（事务边界不变，仅锁时机提前）。

## 遗留

- 无。
