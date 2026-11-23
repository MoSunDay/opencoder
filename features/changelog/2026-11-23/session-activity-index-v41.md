# 会话活跃索引 v41 换形、IdxDelete 自愈与坏行列表降级

一句话：把 `/task` 的活跃索引从 `MAX(updated_at, created_at)` 表达式索引换成普通
`(updated_at DESC, id DESC)` 列索引（schema v41），消除 Turso 0.8.3 回滚后索引
与表行失步导致的 `IdxDelete` 永久写失败；迁移对坏页保持数据库可打开并在后续打开
重试，运行期 append/update 命中该损坏时自动重建索引重试一次，列表在坏页时降级为
覆盖索引取候选 + 点查并跳过不可读行。

## 背景

Turso 0.8.3 在事务回滚/中止时不原子维护索引：表达式索引
`idx_sessions_activity(MAX(updated_at, created_at))` 可能保留比 `sessions` 行
更新的活跃时间。此后每次 `UPDATE sessions SET updated_at = ...` 都会因找不到
索引项报 `SQLITE_CORRUPT`（`IdxDelete: no matching index entry found`），
该会话再也不能持久化消息。坏页还会让整表扫描 / `DROP COLUMN` / 索引重建
整体失败，甚至拖垮 bootstrap。

## 变更

- **schema v41**（`SCHEMA_VERSION` 40 → 41）：索引改为普通
  `idx_sessions_activity ON sessions(updated_at DESC, id DESC)`；`updated_at`
  本身即活跃时间，不再走派生索引维护路径。
- **活跃时间不变量 `updated_at >= created_at`**：创建会话取
  `updated_at.max(created_at)`；`SessionPatch.updated_at` 用
  `MAX(?, created_at)`；消息追加在事务内单调推进 `updated_at`（乱序回填不反复
  写索引）；v41 迁移回填 `updated_at < created_at` 的旧行；`import` 保持不计活跃。
- **迁移与自愈**：迁移先用临时索引探测整表可读性，成功后才删除旧索引；坏库保留
  旧索引并可打开，后续 bootstrap 的 `ensure_index` 重试换形。v40 的
  `DROP COLUMN autopilot_mode` 同样先做可读性探测（坏页不半途重写）。
- **暖打开**：`bootstrap_is_noop` 除版本号外检查 `index_is_current`，索引缺失或
  仍是旧表达式索引时强制完整 bootstrap。
- **运行期修复**：`repair_and_retry` 只识别 `IdxDelete` 型 Corrupt，drop/rebuild
  活跃索引后重试事务一次；已接入消息批量追加与会话更新，其他 Corrupt 原样上抛。
- **列表降级**：快路径用物化 CTE + `INDEXED BY idx_sessions_activity` 按
  `updated_at DESC, id DESC` keyset 分页；遇到不可读存储（`Corrupt` 或 database
  worker `JoinError`）退化为覆盖索引取候选 + 按 id 点查，跳过坏行且不加载
  preview（避免触发 messages b-tree 损坏）。
- **错误分类**：新增 `store::errors`（`is_foreign_key_violation` /
  `is_storage_corruption` / `is_unreadable_storage` / `is_corruption_error`），
  crate 根导出前两个；session 持久化据此识别“会话行已被其他进程删除”。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 落库/更新/追加维持活跃时间 | `orders_by_recent_activity_not_creation`、`append_message_bumps_activity_monotonically` | `crates/store/tests/list_activity_order.rs` |
| 旧库重开（索引被删）自动修复并保持分页 | `existing_schema_reopens_with_activity_pagination_and_unchanged_history` | 同上 |
| keyset 翻页 / 非法 cursor / import 不计活跃 | `cursor_pagination_follows_activity_order`、`invalid_cursor_is_an_error`、`import_does_not_masquerade_as_activity` | 同上 |
| 失步索引 append 自愈 | `append_self_heals_a_desynced_activity_index` | `crates/store/tests/session_activity_index_repair.rs` |
| v40 表达式索引迁移 + 回填 | `migration_v40_rebuilds_expression_index_and_backfills_activity` | 同上 |
| 仅识别 IdxDelete、重建后重试一次、无关错误不重试 | `detects_idx_delete_corruption_through_context`、`repair_and_retry_rebuilds_then_retries_once`、`repair_and_retry_surfaces_unrelated_errors_without_retry` | `crates/store/src/turso_store/conversation/sessions/activity.rs` |
| 旧表达式索引形状识别 | `legacy_expression_index_is_recognized_regardless_of_formatting` | `crates/store/src/turso_store/bootstrap/schema/session_activity.rs` |
| 点查降级与快路径结果一致 / keyset+limit | `point_lookups_reproduce_the_single_statement_page`、`point_lookups_start_after_the_cursor_and_stop_at_the_limit` | `crates/store/src/turso_store/conversation/sessions/listing_tests.rs` |
| FK / Corrupt / worker panic 分类 | `foreign_key_violation_is_detected_through_anyhow_context` 等 3 例 | `crates/store/src/store/errors.rs` |
| schema watermark 同步 v41 | `brain_v4_tables_do_not_increment_current_schema_version` | `crates/store/tests/brain_layered_v4.rs` |

- 测试范围：用户已在提交前完成定向测试；本次按指示免测提交，未重复运行测试或
  Clippy。`brain_layered_v4.rs` 的 watermark 断言随 v41 由 40 机械同步为 41。
- 未运行全量测试；schema 版本升级后旧二进制会以 “schema newer than supported”
  拒绝打开 v41 数据库。
