# turso 回退后的残留清理：rdb 依赖固定回上游 + 文档/记忆引擎口径

一句话：整段回退 rdb 会话存储（见同日 revert 记录）后，收口剩余 rdb
时代痕迹——`rdb` 依赖从本地绝对路径 `/root/rdb` 恢复为上游 git rev
固定；README 双语、rules 与 repo-memory 中的 rdb/RocksDB 措辞、死链
与过时数字修正为 turso 现状。

## 变更清单

- `Cargo.toml` — `rdb = { path = "/root/rdb", … }` 改回
  `git = "…/rdb.git", rev = "e0ee630d…"`；本地 checkout 即该 rev，
  行为不变，仅恢复仓库外/CI 的可复现解析。
- `README.md` — 高性能行改回 Turso WAL 口径，链接真实阈值测试
  `crates/store/tests/store_perf.rs`（`docs/perf.md` 已不存在）；
  性能表下方同一死链同步替换。
- `README.en.md` — `docs/perf.md` 死链替换为 `store_perf.rs`。
- `rules/03-test-pyramid.md` — 示例从不存在的 `LibsqlStore` 修正为
  实际 API `TursoStore::open_memory()`（3 处）。
- `rules/07-project-module-contract.md` — 关联表 schema 号对齐代码
  实际 `SCHEMA_VERSION = 39`。
- `repo-memory.md` — 平台库口径统一为「内嵌 turso 引擎
  （opencoder-database）」；Ontology 行标注独立 `ontology.db`；
  Commit 指针刷新。
- `agents/foundation/event/index.md` — 头部 Commit 指针落盘。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| rdb 上游 rev 固定后可选队列账本可用 | `cargo test -p opencoder-event --features rdb`（9 通过） | `crates/event` |
| 文档链接目标存在 | 人工核验 | `crates/store/tests/store_perf.rs` |

- 测试范围：仅依赖解析与 event 可选 feature 直接链路；文档改动只检查
  内容与链接；未运行全量测试。
