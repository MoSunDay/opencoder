# TUI 启动打开旧版 `opencoder.db` 裸报 File exists 修复

一句话：TUI 启动曾直接以旧 stem `opencoder.db` 作为 rdb 存储路径调用
`create_dir_all`，当旧 turso 构建遗留同名普通文件时，`RdbDb::open` 拿到
裸 `File exists (os error 17)` 直接退出。现在 TUI 与 CLI 共享同一
`open_store_at` 启动序列（legacy-SQLite 迁移检查 → 打开
`opencoder.db.rdb` 目录），且 `RdbDb::open` 对非目录路径改为明确报错。

## 变更清单

- `crates/tui/src/app_helpers.rs` — `open_store` 改为委托
  `opencoder_local::session_cmd::open_store`，与 CLI 启动序列一致。
- `crates/tui/Cargo.toml` — 新增 `opencoder-local` 依赖。
- `crates/local/src/session_cmd.rs` — `open_store` 拆出数据目录核心
  `open_store_at`（建目录 → 迁移检查 → 打开 `opencoder.db.rdb`），
  供 CLI 与 TUI 共用，序列不再漂移。
- `crates/store/src/rdb_store/engine.rs` — `RdbDb::open` 在
  `create_dir_all` 前增加非目录守卫：命中普通文件时返回明确错误
  （提示旧 SQLite 文件需迁移而非打开）；建目录失败也带上路径上下文。

## 行为口径

- TUI 与 CLI 走同一条启动链路：先 `ensure_migrated`（无旧文件为
  no-op），再打开规范目录 `opencoder.db.rdb`；不再触碰旧 stem
  `opencoder.db`。
- 旧 turso 构建遗留的 `<data_dir>/opencoder.db` 普通文件不再被当作
  rdb 目录打开；迁移仍按既有 marker/backup 语义处理，源文件保留原样。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 非目录路径明确报错 | `open_rejects_a_plain_file_path_with_a_clear_error` | `crates/store/src/rdb_store/engine.rs` |
| 共享序列迁移后打开 rdb 目录且幂等 | `open_store_at_migrates_legacy_and_opens_the_rdb_store` | `crates/local/tests/legacy_migration.rs` |
| TUI `open_store` 建出 `opencoder.db.rdb` 目录 | `open_store_creates_db_file_in_workdir_hashed_data_dir` | `crates/tui/src/app_helpers_tests/mod.rs` |

- 定向回归：`cargo test -p opencoder-store --lib engine::tests`、
  `cargo test -p opencoder-local --test legacy_migration`、
  `cargo test -p opencoder-tui --lib` 全部通过。
- Clippy：`cargo clippy -p opencoder-store -p opencoder-local
  -p opencoder-tui --all-targets -- -D warnings` → 零警告。
- 测试范围：仅存储引擎、local 迁移与 TUI 启动链路；未运行全量测试。
