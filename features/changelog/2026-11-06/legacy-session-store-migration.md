# 旧版 SQLite 会话库一次性迁移（local）

一句话：`opencoder-local` 恢复了旧版 SQLite 会话库的迁移能力 ——
`opencoder session migrate-legacy` 子命令 + 启动钩子，把
`<data_dir>/opencoder.db` 一次性迁入 RocksDB 会话库
（`session_store_rdb_dir`），带备份、标记与严格校验；主存储早已是 rdb
（见
[rdb-store-and-event-framework.md](rdb-store-and-event-framework.md)），
本变更只补齐历史数据的入仓通道，不改任何存储实现。

## 行为口径

- **入口**：
  - `opencoder session migrate-legacy [--from PATH] [--backup PATH]
    [--dry-run]`；目标目录固定为当前 workdir 数据目录的
    `session_store_rdb_dir`，不可覆盖；`--from` 仅改源文件并把默认备份
    重定位到源文件旁。
  - 启动钩子：本地 CLI 打开会话库（`open_store`）前先执行
    `legacy::ensure_migrated(&data_dir)`；无旧库文件时为纯 no-op
    （`Ok(None)`），首启行为与全新安装完全一致。
- **提取（只读、严格）**：经 `opencoder_database::maintenance::query`
  打开（`PRAGMA query_only`），逐表 SELECT 固定列序解码；未知枚举串
  （role / event kind / delivery / subagent status）、坏 JSON 列、浮点
  时间戳、非 0/1 标志位都是硬错误，绝不静默改写或丢行。仅迁移仍存在
  于 `SessionMeta` 的列；旧库的 `plan_snapshot`、`plan_input_count`、
  `harness_runtime`、`recorded` 等废弃列带值也不迁移。
  `messages.summary = 1` 的压缩产物行不迁移（可在目标端重建）。
- **装配**：未被 `subagent_tasks.child_session_id` 引用的会话即根，按
  `(created_at, id)` 稳定排序；父子悬挂引用、成环（深度 > 32 或无根
  可达）都是硬错误。逐根经公共 `import_bundle(&store, bundle, None)`
  导入（不直写 rdb KV；workdir_hash 传 `None` 以保留旧 hash）；
  `import_bundle` 自身会重置 compaction/handoff 标记（源 seq 在重排后
  悬空，resume 时重算），迁移遵循该公共语义。逐根导入后核对
  messages / events / 待处理 inputs / subagent 任务数，不符则回滚该根
  并报错。
- **备份与标记**：非 dry-run 在任何写入前创建
  `<source>.pre-rdb.bak`（`VACUUM INTO`；已存在则保留原备份，
  `backup_created=false`），随后原子写
  `<source>.migrated.json`（临时文件 + fsync + rename；format=1，含
  source/destination/backup/counts/migrated_at，无任何会话内容）。
  - 标记有效且目标目录存在 → `already migrated`，直接返回；
  - 标记有效但目标目录缺失 → 报告 stale 并重新迁移；
  - 标记损坏/格式不符 → 硬错误，修复或删除后才可重试。
- **dry-run**：只提取装配出报告，不开目标库、不备份、不写标记。
- **ts 上报口径**：`StoreTarget::LegacyFileOnly` 的文案更新为“旧库可
  经 `opencoder session migrate-legacy` 迁移”，不描述为 rdb 目录。
- **依赖**：`opencoder-local` 新增 `opencoder-database`、`serde` 两个
  workspace 依赖（只用于只读提取与标记 JSON）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 严格解码（枚举/JSON/浮点/标志位） | `parse_message_is_strict_about_every_cell` 等 | `crates/local/src/legacy/parse_tests.rs` |
| 装配：根装配、悬挂引用、环拒绝、排序 | `assembly_builds_one_root_with_nested_child`、`assembly_errors_on_dangling_references_and_cycles`、`assembly_orders_roots_by_creation_then_id` | `crates/local/src/legacy/assembly_tests.rs` |
| 标记读写/状态判定、路径契约、报告渲染 | `marker_roundtrip_and_invalid_states`、`decision_table_for_valid_markers`、`default_paths_follow_the_contract`、`render_report_is_faithful_to_the_run` | `crates/local/src/legacy/tests.rs` |
| 端到端：2 根 + 1 子代理、summary 行排除、计数核对、标记 + 备份 | `migrates_roots_subagent_marker_and_backup_end_to_end` | `crates/local/tests/legacy_migration.rs` |
| 重复执行 no-op | `rerun_after_success_is_a_no_op` | 同上 |
| dry-run 零写入 | `dry_run_performs_no_writes` | 同上 |
| stale 标记重迁且保留原备份 | `stale_marker_re_migrates_and_keeps_original_backup` | 同上 |
| 损坏标记硬错误 | `corrupt_marker_aborts_before_any_write` | 同上 |
| 未知枚举整体中止 | `unknown_enum_cell_aborts_the_whole_migration` | 同上 |
| 悬挂 child 引用中止 | `dangling_child_reference_aborts_instead_of_dropping_rows` | 同上 |
| 启动钩子：无旧库 no-op / 有旧库迁移后可开库 | `startup_hook_is_a_noop_without_a_legacy_file`、`startup_hook_migrates_and_the_store_opens_with_sessions` | 同上 |
| CLI 旗标解析与分发接线 | `cli_flags_parse_with_defaults_and_overrides`、`cli_dispatch_migrates_for_the_current_workdir` | 同上 |

- 定向回归（本次改动范围）：
  - `cargo test -p opencoder-local --lib --quiet` → 106 passed / 0 failed
  - `cargo test -p opencoder-local --test legacy_migration --quiet` →
    11 passed / 0 failed
  - `cargo test -p opencoder-local --test cli_parse --quiet` → 30
    passed / 0 failed
  - `cargo clippy -p opencoder-local --all-targets -- -D warnings` → 通过
