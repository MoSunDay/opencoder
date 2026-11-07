# opencoder-event 可选 rdb 队列账本（event）

一句话：`opencoder-event` 新增 `rdb` feature，在 `src/queue.rs` 提供
RocksDB 持久化 `EventLedger`（事件日志、监听器、分发等待、消费游标）；
默认构建保持纯内存/零存储引擎依赖。撰写时基础存储仍为 turso；主存储
现已切换 rdb（见
[rdb-store-and-event-framework.md](rdb-store-and-event-framework.md)），
`rdb` feature 仍仅服务本 crate 的事件队列，workspace 其余 crate 不因此
激活该 feature。

## 行为口径

- **键布局**（rdb 包装层 slot 前缀之后）：日志 `evt/<seq:020>`（20 位
  零填充，字节序即数值序）、seq 高水位 `evn/last`、消费游标
  `evc/<consumer-id>`、监听器 `evl/<id>`、分发等待 `evw/<execution>`。
- **追加原子性**：seq 分配与日志行在同一 `WriteBatch` 内同步提交，
  无半批数据；重开后续接高水位，seq 严格单调不复用。
- **追加即改写 seq**：账本持有排序权，追加行的 `seq` 由账本分配
  （调用方传入的 seq 不落盘），与 `EventLedger` 重放语义一致。
- **读取**：严格大于游标、按 seq 升序、limit 截断；`limit=0` 返回空。
- **游标校验**：沿用共享纯校验（非空、ascii、无空白、无 `/`、≤128
  字节、非负），并额外拒绝控制字符与 `.`/`..` 形状的 id（持久 key
  安全）。
- **单拥有者**：RocksDB 目录锁拒绝同路径二次打开（跨进程）；进程内
  以异步互斥写租约串行化追加的读-改-写；阻塞调用走 blocking pool。
- **内存模式**：rdb 包装层无内存模式，测试用 tempfile 目录。
- **范围**：基础存储（`opencoder-store` 的 turso/SQLite 后端）不受影响；
  rdb/rocksdb 仅被本 crate 的事件队列使用。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| seq 单调分配 + 严格游标读 + limit | `append_assigns_monotonic_seqs_and_reads_in_order` | `crates/event/tests/rdb_queue.rs` |
| 追加拒绝空 id/kind | `append_rejects_blank_id_and_kind` | `crates/event/tests/rdb_queue.rs` |
| 重复追加产生两行 | `duplicate_append_yields_two_log_rows` | `crates/event/tests/rdb_queue.rs` |
| 日志/游标/计数器重开存活 | `log_and_cursors_survive_reopen_with_monotonic_counter` | `crates/event/tests/rdb_queue.rs` |
| 不安全 consumer id 与负游标拒绝 | `unsafe_consumer_ids_and_negative_cursors_rejected` | `crates/event/tests/rdb_queue.rs` |
| 同路径二次打开拒绝（单拥有者） | `second_open_of_a_live_path_is_refused` | `crates/event/tests/rdb_queue.rs` |
| 监听器 CRUD 持久化与校验 | `listener_crud_is_durable_and_validates_specs` | `crates/event/tests/rdb_queue.rs` |
| 等待精确/前缀匹配、替换、幂等清除 | `waits_match_exact_and_glob_and_clear_idempotently` | `crates/event/tests/rdb_queue.rs` |
| key 零填充使字节序等于 seq 序 | `event_keys_pad_so_byte_order_equals_seq_order` | `crates/event/src/queue.rs` |
| seq 从 1 起步并拒绝耗尽 | `next_seq_starts_at_one_and_refuses_exhaustion` | `crates/event/src/queue.rs` |
| key 安全校验拒绝控制字符与点段 | `key_safety_rejects_control_chars_and_dot_segments` | `crates/event/src/queue.rs` |

- 定向回归：
  - `cargo test -p opencoder-event` → 45 passed / 0 failed（默认构建，
    无 rdb 依赖节点）
  - `cargo test -p opencoder-event --features rdb` → 56 passed / 0 failed
    （45 存量 + 3 内联 + 8 rdb 集成）
- Clippy：`cargo clippy -p opencoder-event --all-targets --features rdb
  -- -D warnings` → 零警告
- 范围：仅本次改动及其直接影响链路；另执行 `cargo check -p
  opencoder-store -p opencoder-local`（通过）。未运行全量测试。
