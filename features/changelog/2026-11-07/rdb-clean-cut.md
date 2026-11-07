# rdb 切换收口：移除全部旧库迁移与兼容层

一句话：rdb（RocksDB）作为唯一会话存储落地后，删除整个 turso/SQLite 时代
的迁移与兼容层——`legacy` 模块（只读提取、marker/backup、幂等导入）、
`session migrate-legacy` CLI、启动迁移钩子、路径策略里的"开发期裸 stem
目录"回退，以及 ts 注册表的旧 `workdir` 标记读取。启动序列只做一件事：
建目录 → 打开规范 `<stem>.rdb` 目录。

## 变更清单

- 删除 `crates/local/src/legacy/`（8 文件）与
  `crates/local/tests/legacy_migration*`、`legacy_schema_variants.rs`。
- `crates/local/src/lib.rs` — 移除 `pub mod legacy` 与
  `SessionSub::MigrateLegacy` 子命令。
- `crates/local/src/session_cmd.rs` — `open_store_at` 去掉迁移钩子，
  仅建目录 + 打开 `session_store_rdb_dir`；删除
  `migrate_legacy_dispatch`/`resolve_migrate_opts`。
- `crates/core/src/data_dir.rs` — 路径策略收敛为唯一规范目录
  `<stem>.rdb`；删除"裸 stem 目录即存储"的开发期布局回退。
- `crates/local/src/ts/` — 注册表扫描只认 `.rdb` 目录；删除旧
  `workdir` 标记文件读取（rdb 存储从不写标记，属死代码）；
  `StoreTarget::LegacyFileOnly` 更名 `PlainFileOnly`，清理/删除提示
  不再指向迁移命令；一次性中央注册表导入更名
  `import_pre_registry_markers`（只读 `.rdb` 存储，与已删除的会话
  迁移无关，命名不再混淆）。
- 文档/记忆：删除迁移类 changelog 三篇；README 双语去掉 Turso 时代
  脚注与"显式迁移命令"口径；repo-memory 与前端 agent 索引改为
  "直接开 `.rdb`"。

## 行为口径

- 会话库启动不再读取、迁移或备份任何旧文件；已有 `<stem>.rdb` 目录
  照常打开，普通文件 `opencoder.db` 永不触碰。
- 机器上遗留的旧 `opencoder.db` 文件成为死数据，可自行删除；不存在
  任何自动读取路径。
- Ontology 的独立 SQLite 后端与主存储切换无关，保持不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 共享启动序列只建目录并打开 `.rdb`，幂等 | `open_store_at_creates_and_opens_the_rdb_store` | `crates/local/src/session_cmd.rs` |
| 路径策略：普通 stem 文件永不当作 rdb 目录 | `rdb_dir_never_collides_with_a_plain_stem_file` | `crates/core/src/data_dir.rs` |
| ts 扫描跳过普通文件 stem 且不建 `.rdb` | `scan_never_opens_a_plain_file_stem` | `crates/local/src/ts/registry.rs` |
| store_target 分类（rdb / 普通文件 / 缺失） | `store_target_classifies_rdb_plain_file_and_missing` | `crates/local/src/ts/actions_tests.rs` |

- 定向回归：`cargo test -p opencoder-core --lib`（329）、
  `cargo test -p opencoder-local --lib`（94）、
  `cargo test -p opencoder-tui --lib open_store`（1）、
  `cargo check --workspace --tests` 全部通过；
  `cargo clippy -p opencoder-core -p opencoder-local --tests` 零警告。
- 测试范围：仅路径策略、启动链路与 ts 注册表扫描；未运行全量测试。
