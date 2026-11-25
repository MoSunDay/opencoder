# store 暖打开探针补齐 guidance 队列形状校验

一句话：warm-open 快路径在 schema 水位已是当前值时跳过 bootstrap，导致 legacy
`execution_report_queue`（无 `eligible` 列）的修复不再执行，`pending_reports`
等查询直接 `no such column: eligible`；只读探针 `bootstrap_is_noop` 现同时校验
队列 `eligible` 列与 `execution_reports_ready` 索引，legacy 形状重新走完整
bootstrap 修复。

## 背景

- 暖打开快路径（`9e2ee6af2`）在 schema 版本等于当前值、活跃索引为现行形状且
  runtime 表齐备时跳过写锁引导与 checkpoint，避免短命令争抢跨进程写锁。
- `reporting::initialize` 是唯一补 `eligible` 列、回填并建部分索引的位置，但它
  只在完整 bootstrap 内运行；legacy 队列遇上当前水位时被探针误判为 no-op，
  修复永远不触发。
- CI `project-store-tests` 的 store 契约 job 因此变红：
  `brain_layered_v4::reporting::legacy_guidance_queue_migrates_to_indexed_heads_and_ack_is_atomic`
  稳定失败（`Parse error: no such column: eligible`）。

## 变更

- `crates/store/src/turso_store/execution/reporting/mod.rs`：新增
  `schema_is_current`（`PRAGMA table_info` 校验 `eligible` 列 +
  `sqlite_master` 校验 `execution_reports_ready` 索引）。
- `crates/store/src/turso_store/bootstrap/schema/parts/part_1.rs`：
  `bootstrap_is_noop` 在 runtime 表齐备后调用该探针；legacy 队列使 bootstrap
  重新成为真实步骤。
- CI 命名清理：`project-store-tests` 的 `libsql` job / 日志 / artifact 改名为
  `turso` / `store`（依赖早已全量切换 turso，Cargo.lock 无 libsql 包）。
- Windows 平台门禁按用户决策暂时移除（`platform.yml` 仅保留 macOS gate）。

## 功能 → 测试

| 功能 | 测试 |
|------|------|
| legacy guidance 队列迁移（加列、回填、部分索引、ack 原子性） | `opencoder-store` `tests/brain_layered_v4.rs::reporting::legacy_guidance_queue_migrates_to_indexed_heads_and_ack_is_atomic` |
| 暖打开探针不误判 legacy 队列 | 同上（重开即修复）+ `opencoder-store` `--lib` |
| 既有打开 / 迁移 / 并发回归 | `opencoder-store` `--test schema_bootstrap --test store_migrations --test connection_lifecycle --test concurrent_serialized` |

## 实测记录

- 修复前：`cargo test --locked -p opencoder-store --test brain_layered_v4` →
  1 failed（`no such column: eligible`）。
- 修复后：同命令 → 13 passed / 0 failed；
  `cargo test --locked -p opencoder-store --no-fail-fast` → exit 0（0 failed）；
  `--lib` → 87 passed；`schema_bootstrap` / `store_migrations` /
  `connection_lifecycle` / `concurrent_serialized` 全部通过。
- `catalog_maintenance` 2 项需系统 `python3` 带 `_sqlite3`：本机默认
  `/usr/local/bin/python3` 缺该模块，`PATH=/usr/bin:...` 前置后 2 passed（CI
  ubuntu-latest 自带 sqlite3，不受影响）。
- `cargo fmt --all -- --check` → 通过；
  `cargo clippy --locked --workspace --all-targets -- -D warnings` → 0 warnings。
- 测试范围：本次修改及其直接影响链路；未运行全量 workspace 测试。
