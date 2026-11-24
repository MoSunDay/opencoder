# 存储不可读时消息追加降级落地 + `db_repair` 页级修库工具

一句话：损坏库里 `messages` 根页被索引内部页覆盖（`sqlite3 .dump` 导出 0 条消息、
按 rowid 读写 panic、回填活跃时间报 `inconsistent overflow chain`），应用层只能
"能启动、能列会话、能降级写"，历史消息必须重建文件才能找回。本次：① 追加消息时
活跃索引重建也撞上不可读存储，则跳过活跃时间戳把消息写进去（而不是整段丢失），
且运行期重建改为"先建探针索引、整表可读再换名"，不再依赖 Turso 回滚 DDL；
② 新增 `scripts/maintenance/db_repair/`，用 `.dump` + 全页扫描 carve 按 B-tree 叶页
单元记录恢复 `messages`/`session_events`，在副本上重建出一个 `integrity_check` 通过
的新库；`--output` 不得指向源库、失败保留工作副本、索引页孤儿行按会话继承 `seq`。
实测 45,272 条索引项恢复 45,211 条消息、补回 42,562 条事件，新二进制在其上
`session list` 带预览、`session show` 正常。

## 背景

- c80f421da 之后，损坏库已可启动（v41 迁移对坏行逐行回填并跳过、列表降级点查），
  但 `append_many` 的 `repair_and_retry` 在索引重建（`UPDATE sessions` / `CREATE INDEX`
  全表扫描）再次撞上不可读行时直接返回错误—— assistant 回复整段落库失败。
- 取证结论（一个约 2.4GB、schema v39、freelist 为 0 的损坏副本）：`messages`
  根页被一个 0x02 索引内部页覆盖，Turso 对该表不可读不可写
  （`pager.rs:507` panic）；但消息行仍完整地散落在 0x0D 表叶页里，可按 rowid 恢复，
  另有一批只存在于 0x0A 索引叶页（无 rowid）。`sessions` 中有一行的
  `task_type`/`handoff_plan` 为截断 overflow（非法 UTF-8，Turso 打开即报
  `TEXT value contains invalid UTF-8`）；若干 `sub-*` 会话行已删而消息仍在。
- 根因不是代码缺陷：多个 opencoder 进程以 `experimental_multiprocess_wal(true)`
  并发写同一文件造成页级丢失更新。应用代码中没有任何 `DELETE FROM messages`。

## 变更

- **`sessions/activity.rs`**：新增 `repair_and_retry_or(work, degraded)`，骨架抽为
  `repair_with(work, degraded, rebuild)`：`IdxDelete` → 重建索引；重建成功重试原体；
  重建失败且 `is_unreadable_storage` → 跑 `degraded`（warn 记录）；其他重建错误原样
  返回。`rebuild` 注入以便单测驱动失败分支而不需要真损坏库。
- **`sessions/activity.rs` 重建顺序**：`rebuild_activity_index` 原为同一事务内
  `DROP INDEX` 再 `CREATE INDEX`，CREATE 撞上坏行后索引是否存在完全取决于 Turso 是否回滚
  DDL——而本模块存在的前提就是 Turso 回滚不原子；列表两处 `INDEXED BY
  idx_sessions_activity` 没有索引缺失的回退。现改为与 v41 迁移相同的探针式：先在
  `BEGIN IMMEDIATE` 内建 `idx_sessions_activity_rebuild`（失败即语句级回滚、旧索引不动），
  成功后才在一个事务内 DROP 旧索引 + CREATE 正式索引 + DROP 探针；换名事务若仍失败，
  尽力 `CREATE INDEX IF NOT EXISTS` 恢复列表索引再返回错误。实测 fixture（截断 overflow
  链的会话行）下旧实现的 ROLLBACK 也能恢复索引，新实现只是去掉对这一行为的依赖。
- **`messages/parts/part_1.rs`**：`append_chunk_in_tx(conn, session_id, msgs, touch)`；
  `append_many` 用 `repair_and_retry_or`，降级分支 `touch=false` 不执行
  `UPDATE sessions SET updated_at`，索引列不动，写入不再依赖活跃索引（会话在 `/task`
  中保持旧位置，直到修库）。
- **`scripts/maintenance/db_repair/`**（纯函数、无类、`/usr/bin/python3` 运行）：
  - `carve.py`：`varint`/`decode_record`/`local_size`/`read_payload`/`leaf_cells`，
    按列数与类型签名识别 `messages`(14 列)/`session_events`(6 列) 单元；0x0D 页取
    rowid 为 `seq`，0x0A 页 `seq=None`。
  - `repair.py`：`snapshot`（复制 db+wal 到 `.repair-<name>/`，源库只读）→ `sqlite3 .dump`
    → 把结尾 `ROLLBACK; -- due to errors` 改成 `COMMIT`、`INSERT INTO` 改
    `INSERT OR REPLACE INTO`（陈旧页副本不再中断导入）→ `sqlite3 -bail` 导入 → 单次
    mmap 全页扫描补行（BATCH 20000）→ 无 rowid 行按 `created_at` 继承索引中未用 `seq`
    → 孤儿会话补 stub 行 → 非法 UTF-8 TEXT lossy 清洗 → `task_type`/`kind` 规整 →
    `sqlite_sequence` 抬水位 → `integrity_check` + 统计 → JSON 报告。
  - 评审后加固：`--output` 与 `--database` 为同一文件时拒绝（否则 `load_dump` 会先删掉
    源库及其 `-wal/-shm/-tshm`）；运行中途失败保留 `.repair-<name>/` 副本与 dump，成功才删；
    索引页孤儿行按 `(session_id, seq)` 读取旧索引，各会话内按 `created_at` 对位继承
    未用 `seq`（纯函数 `assign_orphan_seqs`），不再全局按时间猜。
  - README 给出完整步骤：停进程 → 备份 → 修库 → 核对报告 → 替换（删 `-wal/-shm/-tshm`）→ 启动。

## 实测（损坏副本 → 重建库）

| 项 | 结果 |
|----|------|
| `.dump` | 12,846,111 条 INSERT，`messages` 0 行（预期），末尾 ROLLBACK 被改写为 COMMIT |
| carve | 带 seq 消息 44,732；无 rowid 479（19 个重复 id）；补回事件 42,562 |
| stub 会话 | 4 个 `sub-*` 会话（消息仍在、会话行已丢失）各补一条最小行 |
| 清洗/规整 | `sessions.handoff_plan` 1、`task_type` 1（lossy）；`task_type` 规整 1 |
| verify | integrity ok；sessions 368、messages 45,211（id 唯一）、session_events 12,887,252；`sqlite_sequence` messages 12849113 / events 12889630 |
| 新二进制 | v39→v41 迁移成功；`session list` 带预览 0.2s；`session show` 68 行；append 新 seq 12849114+ 无冲突 |

两次独立运行（复用 dump / 从头 dump）报告逐项一致。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 重建撞上不可读存储时跑降级体 / 其他重建错误原样返回且不降级 / 重建成功重试原体 | `degraded_body_runs_when_the_rebuild_hits_unreadable_storage`、`other_rebuild_failures_surface_without_running_the_degraded_body`、`successful_rebuild_retries_the_original_body` | `crates/store/src/turso_store/conversation/sessions/activity.rs` |
| 真实形状：索引失步 + 另一会话行 overflow 链截断 → 追加落库、不刷活跃时间、列表索引保留且无探针残留 | `append_lands_without_activity_touch_when_the_index_cannot_be_rebuilt` | `crates/store/tests/session_activity_index_repair.rs` |
| 端到端重建（损坏页 → 导入 → 补行 → stub → 规整）/ ROLLBACK→COMMIT 改写 / 非法 UTF-8 清洗 / 孤儿行按会话对位 seq / 拒绝覆盖源库 / 失败保留工作副本 | `test_rebuild_restores_orphaned_messages_and_events`、`test_finalize_dump_only_patches_the_error_rollback`、`test_scrub_rewrites_text_that_is_not_utf8`、`test_assign_orphan_seqs_pairs_each_session_with_its_own_index_seqs`、`test_repair_refuses_to_overwrite_the_source_database`、`test_failed_repair_keeps_the_scratch_copy` | `scripts/maintenance/db_repair/test_repair.py` |

- `cargo test -p opencoder-store --test session_activity_index_repair --test list_activity_order` 9/9，`--lib activity` 7/7；`PYTHONPATH=scripts/maintenance /usr/bin/python3 -m unittest db_repair.test_repair -v` 6/6 OK。
- 收尾复跑：`cargo clippy -p opencoder-store --all-targets -- -D warnings` 零警告（顺手修复 `listing.rs` 的 `push_str("1")` 与 `sessions/parts/part_2.rs` 文档段落行首 `+` 触发的 `doc_lazy_continuation`，均为既有问题）；上述 Rust 9/9 与 Python 6/6 复跑结果一致。
- 新集成测试的 fixture 做法：会话 s2 带 20 KiB 标题产生 overflow 链，把首页 overflow 的 next 指针清零（sqlite3 `integrity_check` 报 `overflow list length is 1 but should be 4`）。注意 Turso 下 `SELECT updated_at … WHERE id=?` 被活跃索引完全覆盖、不读表行，fixture 断言必须读 `title`，且错误在 `rows.next()` 而非 `query()` 才出现。
- `cargo test -p opencoder-store --no-fail-fast` 仅 2 个既有失败与本改动无关：
  `catalog_maintenance`（调用的 `python3` 3.13 无 `_sqlite3`）、
  `brain_layered_v4::reporting::legacy_guidance_queue_migrates_to_indexed_heads_and_ack_is_atomic`
  （`no such column: eligible`，warm-open `bootstrap_is_noop` 快路径跳过了测试手工建的 legacy 队列表迁移）。
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。

## 已知限制

- 所在页已被覆盖的消息与 `session_events` 尾部记录无法恢复（数量见报告的 `index_only` / 未匹配条目）；
  ghost 会话以 stub 行复活（如确为有意删除可再删）；修库不回填 `updated_at`，恢复出的会话保留旧版本。
- 修库不消除根因：多进程并发写同一库会再次损坏，应减少并发写入或升级存储引擎。
