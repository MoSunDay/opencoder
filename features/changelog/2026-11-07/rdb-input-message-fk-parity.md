# rdb 补回 session_inputs/messages 对 sessions(id) 的外键语义

一句话：rdb（RocksDB）后端此前遗漏了旧 SQLite `session_inputs` 与
`messages` 对 `sessions(id)` 的外键语义——向不存在的 session admit
输入或 append 消息会静默成功，导致 TUI steer 图像在存储失败时被
当作成功而丢弃。现在这两个入口对缺失 session 以
`FOREIGN KEY constraint failed` 拒绝，与 brain vectors/tokens 等域
既有复刻口径一致。

## 变更清单

- `crates/store/src/rdb_store/conversation/inputs.rs` — `admit` 在
  UNIQUE 检查之后、落行之前校验 session 存在；`admit_once` 仅在
  插入路径（dedup 索引未命中之后）做同一校验，保持 SQL 顺序：
  UNIQUE/dedup 解析先于外键失败。
- `crates/store/src/rdb_store/conversation/messages.rs` —
  `append_many` 对非空批校验 session 存在（空批在 SQL 中不会触
  发外键，保持 no-op）。

## 行为口径

- `admit_input` / `admit_input_once`（插入路径）对不存在的
  session 返回 `insert input: FOREIGN KEY constraint failed`；
  idempotent 命中与 `InputConflict` 语义不变。
- `append_messages` 对不存在的 session 且非空批返回
  `insert message: FOREIGN KEY constraint failed`；空批照旧成功。
- 迁移链路（先导 sessions 再导 messages/inputs）不受影响。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 缺失 session 的 admit/`admit_input_once` 外键拒绝 | `admit_into_missing_session_replicates_the_foreign_key_failure` | `crates/store/tests/rdb_conversation/suite_inputs.rs` |
| 缺失 session 的 append 外键拒绝、空批 no-op | `append_into_missing_session_replicates_the_foreign_key_failure` | `crates/store/tests/rdb_conversation/suite_messages.rs` |
| steer 图像在存储失败时不被丢弃 | `store_failure_preserves_pending_images` | `crates/tui/src/subagent_input.rs` |

- 定向回归：`cargo test -p opencoder-store --test rdb_conversation`、
  `cargo test -p opencoder-local --test legacy_migration`、
  `cargo test -p opencoder-tui --lib` 全部通过。
- Clippy：`cargo clippy -p opencoder-store -p opencoder-tui
  --all-targets -- -D warnings` → 零警告。
- 测试范围：仅 rdb 会话域外键复刻及其直接影响链路（TUI steer、
  local 迁移）；未运行全量测试。
