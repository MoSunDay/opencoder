Commit: (working-tree, pre-initial-commit)

# fix(store): run_tx helper replaces libsql::Transaction to eliminate panic-on-drop

## 背景
libsql 0.9.30 的 `Transaction::Drop` 实现调用 `do_rollback().unwrap()`——
当 rollback 失败时直接 panic。OpenCoder 的 `LibsqlStore` 用单个共享 Connection +
`db_lock: tokio::sync::Mutex<()>` 串行化所有 26 个 async Store 方法。当
`tokio::select!` 取消一个进行中的事务时，drop 顺序不保证：`MutexGuard` 可能先于
`Transaction` 被 drop，此时另一任务已获取锁并修改共享连接，使 `Transaction::Drop`
的 rollback 命中 `SqliteFailure(1, "cannot rollback - no transaction is active")`
→ `unwrap()` panic → 整个进程崩溃。

## 变更
### 新增
- **`crates/store/src/libsql_store/tx.rs`**（新文件）：`run_tx<F, Fut, T>(conn,
  begin_sql, work)` 纯异步函数，用手动 `BEGIN`/`COMMIT`/`ROLLBACK` 替代
  `libsql::Transaction`。语义：
  - `BEGIN` 前做 best-effort `ROLLBACK`（恢复上一轮取消遗留的悬挂事务，
    无活动事务时该语句是可忽略错误）。
  - `work` 返回 `Ok` → `COMMIT`（失败用 `context` 附加上下文返回 `Err`）。
  - `work` 返回 `Err` → `ROLLBACK`，失败时 `tracing::warn!` 记录并吞掉
    （**绝不 panic**），原错误照常返回。
  - `begin_sql` 参数：`"BEGIN"`（deferred，默认）或 `"BEGIN IMMEDIATE"`（写锁，
    仅供 `claim_next_queue`）。

### 改动
- **`crates/store/src/libsql_store/inputs.rs`**：5 处 `libsql::Transaction`
  → `run_tx`（`admit_input`、`pending_inputs` 内的事务体、`promote_inputs`、
  `claim_next_queue`（`"BEGIN IMMEDIATE"`）、`swap_input_order`）。
- **`crates/store/src/libsql_store/messages.rs`**：2 处 → `run_tx`
  （`append_many`、`import`）。
- **`crates/store/src/libsql_store/events.rs`**：1 处 → `run_tx`
  （`append_many`）。
- **`crates/store/src/libsql_store/mod.rs`**：声明 `mod tx;` 并 `pub(crate) use`。
- **`crates/store/src/libsql_store/schema.rs`**：新增 `add_column_if_absent` 辅助函数（检查 `PRAGMA table_info` 跳过已存在列），全部 `ALTER TABLE ... ADD COLUMN` 迁移（v2 `sse_kind`、v3 `handoff_seq`/`handoff_plan`/`skill`）改用此函数，消除 `CREATE TABLE` 已含全量 schema 但 `schema_version` 记录旧版本时的 `duplicate column name` 错误。

### 不变
- `Store` trait 公共 API 零变更——`run_tx` 为私有辅助函数。
- 事务语义等价（`"BEGIN"` = `BEGIN DEFERRED`；`"BEGIN IMMEDIATE"` 保留）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 取消事务不 panic + 后续操作正常 | `cancelled_transaction_does_not_panic` | tests/store_integration.rs |
| 取消后并发操作保持一致性 | `cancelled_then_concurrent_ops_stay_consistent` | tests/store_integration.rs |
| 迁移幂等（列已存在不报错） | `schema_migration_is_idempotent_when_column_already_exists` | tests/store_integration.rs |

- `cancelled_transaction_does_not_panic`：用 `tokio::select!`（biased，1ms sleep）
  取消一个 50 条消息的批量写入，随后写入 3 条消息并断言全部持久化。对取消的两种
  结果（批量已 commit 或 rolled-back）均鲁棒；核心断言是「不 panic」+ 后续操作成功。
- `cancelled_then_concurrent_ops_stay_consistent`：10 轮「取消 5 条批量写入 → 立即
  成功写入 1 条」交替，断言全部 10 条 `ok{round}-0` 存活。

- 全量回归：`cargo test -p opencoder-store` → 40 passed; 0 failed
  （3 lib unit + 1 concurrent_serialized + 5 inputs_integration + 28 store_integration
  + 3 store_perf）
- clippy：`cargo clippy -p opencoder-store --all-targets -- -D warnings` → 零警告
- 行数：`tx.rs` 56 行（新文件 ≤ 400）

## Impact Surface
- 用户：修复了一个在异步取消（如 `tokio::select!` 中止 drain、cancel subagent）时
  可能导致进程 panic 崩溃的潜在路径，提升运行时稳定性。
- 兼容性：公共 API 不变，下游 session/web/cli 零改动。
- 回滚语义：与原 `libsql::Transaction` 等价（deferred/IMMEDIATE 保留）；唯一行为
  差异是 rollback 失败时从 panic 降级为日志警告。

## Related Docs
- [agents/store](../../../agents/store/index.md)
- [rules/01-mandatory-tests.md](../../../rules/01-mandatory-tests.md)
