Commit: (working-tree)

# 项目团队测试数据隔离修复与残留清理

## 背景

- 项目待办的 team 执行器集成测试（`crates/project/tests/executor_team_dag_brain.rs`）以 `tempfile::tempdir()` 作为 workdir，但执行器 team 驱动的 team_root 缺省回填规则（`crates/project/src/executor/team_drive.rs` 的 `team_root_for`：`config.team_root` 等于缺省值时挂到 `opencoder_core::data_dir_for(workdir)/team`）把团队数据物化到真实数据根。tempdir 销毁后，`/data00/opencoder-data`（由 `/root/.local/share/opencoder` 符号链接指向）下按 tempdir 哈希命名的孤儿目录永久累积，共 88 处 `project-t-team` 残留（含 team.json、话题 plan/result/summary）。
- 注意：GNU find 不跟随作为起始点的符号链接，早期用 find 直接搜 `/root/.local/share/opencoder` 会误报为 0，必须用 `find -L` 或基于真实路径 `/data00/opencoder-data` 统计。

## 修复（仅测试夹具，生产代码零改动）

- `harness_on` 在创建 tempdir 后写入项目级配置 `<workdir>/opencoder.json`，内容 `{"team_root": "<fixture>/team"}`；`Config::load` 经 `config_candidates` 读取 workdir 下的 `opencoder.json` 并按 key 合并，`team_root_for` 检测到非缺省值后直接采用，全部团队数据落进 fixture 随 tempdir 自动清理（与既有 `archive_root` 覆盖同一思路）。
- 同步把断言中 team_root 从 `opencoder_core::data_dir_for(&h.dir).join("team")` 改为 `h.dir.join("team")`。

## Validation（功能 → 测试名）

- 团队数据留在 fixture 内：`crates/project/tests/executor_team_dag_brain.rs::team_executor_runs_inline_spec_to_completion`（topic 目录断言改为 fixture 内路径）。
- 全 crate 回归：`cargo test -p opencoder-project`，6 个套件 32+4+3+8+15+0 共 62 项全部通过，0 失败。
- Lint：`cargo clippy -p opencoder-project --all-targets -- -D warnings` 零告警。
- 污染计数：清理并修复后重复运行测试，`/data00/opencoder-data` 下 team 目录与 team.json 计数前后均为 0。

## 数据清理（运维）

- 删除 `/data00/opencoder-data` 下 88 个内容仅为 `project-t-team` 的 `team/` 目录及随后清空的 88 个哈希目录；校验 `find -L /data00/opencoder-data` 下 `project-t-team` / `team` / `team.json` 均为 0。
- 控制面（opencoder-server control.db）fleet_definitions 中 6 个 `release-*` 发布验收团队定义此前已清空（`GET /api/teams` 返回 `{"teams":[]}`），删除前备份 `/tmp/control-backup-20260916-101824.db`。

## Related Docs

- [项目模块](../../../agents/project/index.md)
- [团队模块](../../../agents/team/index.md)
