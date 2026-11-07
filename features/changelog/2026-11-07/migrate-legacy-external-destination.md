# `session migrate-legacy --from` 外部源把 destination 重定基到源目录

一句话：`--from` 指向其它目录的 legacy 库时，marker 与 backup 写在源
目录，但导入的数据却落进**当前 workdir** 的 `opencoder.db.rdb`，三者
分裂：源目录留下一个 destination 指向别处的 stale marker，外部会话
混入当前 workdir 的存储。现在显式 `--from` 且不等于默认源时，
destination 一并重定基为 `<源目录>/opencoder.db.rdb`，marker/backup/
数据三者同目录。

## 变更清单

- `crates/local/src/session_cmd.rs` — 迁移选项组装抽成纯函数
  `resolve_migrate_opts(data_dir, from, backup, dry_run)`；外部源时
  source/derived backup/destination 一起随源重定基。

## 行为口径

- 无 `--from`：与之前完全一致（默认源/备份/目标均在当前 workdir 数据
  目录）。
- `--from` 显式等于默认源：destination 不变（同一路径）。
- `--from` 指向外部目录：destination = `<外部目录>/opencoder.db.rdb`，
  不再污染当前 workdir 存储；源目录中指向别处的旧 marker 会被
  `legacy::run` 按 stale marker 语义自动重迁。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 默认选项落在当前数据目录 | `migrate_opts_default_to_the_active_data_dir` | `crates/local/src/session_cmd.rs` |
| 外部源三者随源重定基 | `migrate_opts_rebase_destination_next_to_an_external_source` | `crates/local/src/session_cmd.rs` |
| 显式默认源不改变 destination | `migrate_opts_keep_default_destination_for_explicit_default_source` | `crates/local/src/session_cmd.rs` |

- 定向回归：`cargo test -p opencoder-local --lib session_cmd`（15）通过。
- 真机复核：外部老 schema 库（`1025aa…`）重迁后 marker/backup/`.rdb`
  同目录、stale marker 自动重迁。
- 测试范围：仅 migrate-legacy 选项组装链路；未运行全量测试。
