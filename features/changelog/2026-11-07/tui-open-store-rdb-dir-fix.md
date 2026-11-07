# TUI 启动打开旧 stem 报 File exists 修复

一句话：TUI 启动曾直接以 `opencoder.db` 作为 rdb 存储路径调用
`create_dir_all`，当该路径已存在同名普通文件时报裸
`File exists (os error 17)` 退出。现在 TUI 与 CLI 共享同一
`open_store_at` 启动序列（建目录 → 打开规范 `opencoder.db.rdb` 目录），
且 `RdbDb::open` 对非目录路径报明确错误。

## 变更清单

- `crates/tui/src/app_helpers.rs` — `open_store` 委托
  `opencoder_local::session_cmd::open_store`，与 CLI 启动序列一致。
- `crates/tui/Cargo.toml` — 新增 `opencoder-local` 依赖。
- `crates/local/src/session_cmd.rs` — `open_store` 拆出数据目录核心
  `open_store_at`（建目录 → 打开 `opencoder.db.rdb`），CLI 与 TUI 共用，
  序列不再漂移。
- `crates/store/src/rdb_store/engine.rs` — `RdbDb::open` 在
  `create_dir_all` 前增加非目录守卫（命中普通文件返回明确错误）；
  建目录失败带路径上下文。

## 行为口径

- TUI 与 CLI 走同一条启动链路，直接打开规范目录
  `opencoder.db.rdb`；普通文件 `opencoder.db` 永不被当作 rdb 目录打开。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 非目录路径明确报错 | `open_rejects_a_plain_file_path_with_a_clear_error` | `crates/store/src/rdb_store/engine.rs` |
| 共享序列建出并打开 rdb 目录且幂等 | `open_store_at_creates_and_opens_the_rdb_store` | `crates/local/src/session_cmd.rs` |
| TUI `open_store` 建出 `opencoder.db.rdb` 目录 | `open_store_creates_db_file_in_workdir_hashed_data_dir` | `crates/tui/src/app_helpers_tests/mod.rs` |

- 定向回归：`cargo test -p opencoder-local --lib`、
  `cargo test -p opencoder-tui --lib open_store` 通过。
- 测试范围：仅 local/TUI 启动链路与存储引擎守卫；未运行全量测试。
