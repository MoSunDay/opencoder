# 测试组织规则与夹具去重

## 概述

- 新增 [rules/11-test-organization.md](../../../rules/11-test-organization.md)：模块 → 测试位置地图、目录结构约定（功能域目录目标 / 同名目录 `#[path]` 拆分 / `tests/support/mod.rs` 夹具标准命名）、新增 case 决策树、验收脚本归组约定；[rules/README.md](../../../rules/README.md) 索引与检查清单、[repo-memory.md](../../../repo-memory.md) 仓库规则节同步。
- `crates/session/tests/support/mod.rs` 新建标准夹具模块：`memory_store()` + `agent_with(mock)` 纯函数夹具；`tests/steer_followup.rs` 作为触碰文件迁移示范改用 `support::memory_store`（存量 `common/`、`harness/` 与其余 ≈108 个平铺文件不迁移，触碰时再归队）。
- `crates/node/tests/runner_dag.rs` 内联 DAG wire-stub（376 → 222 行）移入共享 `crates/node/tests/support/dag.rs`（159 行），复用 `support/` 的 Bearer `auth` 中间件与 `wait_for`，删除内联重复。
- 明确不做：不批量迁移平铺测试、不新建 workspace 级测试支撑 crate（rules/11 记为未来选项）、测试不登记进 Ontology（rules/09 界定能力目录）。

## 测试覆盖

| 改动 | 验证方式 |
|------|----------|
| rules/11 + rules/README + repo-memory（纯文档） | 内容与相对链接检查（链接目标均存在）；`wc -l rules/11-test-organization.md` = 96 ≤ 400 |
| session 标准夹具 + steer_followup 迁移 | `cargo test -p opencoder-session --test steer_followup` → 8 passed / 0 failed |
| node runner_dag stub 去重 | `cargo test -p opencoder-node --test runner_dag` → 2 passed / 0 failed |
| 两 crate Clippy | `cargo clippy -p opencoder-session --all-targets -- -D warnings`、`cargo clippy -p opencoder-node --all-targets -- -D warnings` → 零警告 |

- 测试范围：本次改动及其直接影响的测试目标；未运行全量测试（规则 02）。
- 未修改 Rust 生产代码，仅测试代码与文档。
