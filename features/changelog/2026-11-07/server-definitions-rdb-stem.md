# control bootstrap 的 definitions 目录固定走 `.rdb` 后缀

一句话：control bootstrap 侧此前仍以旧 turso stem `definitions.db`
作为路径打开共享 definitions catalog；升级安装上该路径是普通 SQLite
文件时，`RdbDb::open` 的 `create_dir_all` 会以裸 `File exists
(os error 17)` 失败（与已修复的 TUI session-store 同一类问题）。现在
bootstrap 统一通过 `definitions_rdb_dir` 打开
`definitions.db.rdb` 目录，与 `FleetStore`/`TsRegistry` 的后缀策略
一致。

## 变更清单

- `crates/control/src/bootstrap.rs` — 新增纯函数
  `definitions_rdb_dir(data: &Path) -> PathBuf`，固定返回
  `data.join("definitions.db.rdb")`；`new_state`、
  `new_state_with_projects`（test seam）、`state_with_projects`、
  server 启动路径共 4 处 `RdbStore::open_catalog` 调用全部改走该
  helper，仓库内不再有裸 stem `definitions.db` 调用点。

## 行为口径

- 4 处调用点行为一致：永远打开 `definitions.db.rdb` 目录，不再把旧
  turso 遗留的 `definitions.db` 普通文件当 rdb 目录打开，升级安装
  不会再触发 `File exists (os error 17)`。
- 注意：旧 turso `definitions.db` 的内容不会自动迁移——目录级
  catalog 随 ontology 同步重建；如需保留旧内容需另行迁移。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 无裸 stem 调用点（grep 守卫口径） | `grep -rn 'definitions\.db' crates --include='*.rs' \| grep -v '\.rdb'` 为空 | — |
| server bootstrap 链路（infra 静态启动） | `infra_static` 套件 | `crates/control/tests/e2e/infra_static.rs` |
| definitions catalog 经 bootstrap 打开 | `teams_dag_defs` 套件 | `crates/control/tests/e2e/teams_dag_defs.rs` |

- 定向回归：`cargo test -p opencoder-control --test e2e
  infra_static` → 9 passed / 0 failed；`cargo test -p opencoder-control
  --test e2e teams_dag_defs` → 10 passed / 0 failed。
- 静态检查：`cargo check -p opencoder-control --tests`、
  `cargo clippy -p opencoder-control --all-targets -- -D warnings`
  均零告警。
- 测试范围：仅 control bootstrap 及其直接影响的 e2e 启动链路；未
  运行全量测试。
