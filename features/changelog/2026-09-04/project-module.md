# 项目管理模块（opencoder-project + /api/project + SPA「项目」tab）

## 动机

TODO 工作流（todos crate）是 LLM 自治的：父会话自己拆解、执行、验收。用户缺少一个**手工策展**的层次——目标 → 里程碑 → 待办——来主导方向，再借 agent 能力把每个待办做实：粗略草稿 → plan agent 生成完整实施方案 → act agent 执行落地，可反复执行、版本留痕、随时取消。

## 落地

- **core**：`StorageBackend`/`StorageConfig`（`crates/core/src/config/storage.rs`，环境变量 `{VAR}` 展开），`Config.storage.project` 指定项目数据后端（libsql 默认 / mysql / starrocks feature 二选一）。
- **store**：`ProjectStore` trait（18 方法）+ `project_types.rs`（goal/milestone/todo/run）+ `open_project_store` 工厂；libsql 实现四表（schema v15：goals / milestones→goals CASCADE / todos→milestones SET NULL 可空即 backlog / todo_runs 留痕）；`sql_store/` feature-gate MySQL/StarRocks 后端（DDL 方言化 + text 协议，StarRocks prepared SELECT 旧快照问题规避）。会话/消息仍走 `Arc<dyn Store>`。
- **project crate**：`ProjectService`（OnceLock Deps + spawns 取消注册表）：plan 每次新建 plan-agent 会话生成 `plan_md`；execute resume 同一 `active_session_id` 持续推进（「新或续」策略），每次执行 `project_todo_runs` version 自增留痕；取消收敛 run cancelled + todo 回 planned；失败 todo failed 不滞留 running。todo 状态机 `draft→planned→running→done|failed` 服务层独占，web PATCH 不暴露 status/plan_md。
- **web**：`/api/project/*`（goals/milestones/todos CRUD + overview + plan/execute/cancel/runs），`AppState.project` 未 init 全 503。
- **SPA**：「项目」tab 四视图（目标/里程碑/待办/总览）+ todo 抽屉（plan/run 历史）+ markdown 预览（marked）。

## 关键取舍

- 不复用 todos 编排（无 candidate 门禁/重试），复用 session 直驱范式（`run/resume` + 事件 flusher），运行会话可在会话交互页回放。
- 硬 cancel 后 `opencoder_session::run` 可能返回 `Ok(())` 且无新 assistant 消息（空 turn 丢弃）——`Ok+无输出+cancel.is_cancelled()` 同样收敛为 cancelled，避免悬挂 running。
- StarRocks：全部语句 text 协议（`raw_sql` 内联参数；sqlx 0.8.6 `RawSql::fetch_optional` 误委托 fetch_one，用 fetch_all 恢复 optional）；级联删除顺序执行；测试 `eventually()` 轮询。

## 测试清单

| 层 | 文件 | 用例 |
|---|---|---|
| store（libsql） | `crates/store/tests/project_store.rs` | 8：CRUD/级联/状态流转 |
| store（sqlx，env 门控） | `crates/store/tests/sql_project_store.rs` | 2（`OC_TEST_MYSQL_DSN`/`OC_TEST_STARROCKS_DSN`，真容器实测过） |
| project 单元 | `crates/project/src/{context,service}.rs` 内联 | 6：prompt 组装 4 + service 未初始化/未知取消 2 |
| project 集成 | `crates/project/tests/plan_and_execute.rs` | 5：plan 回写/执行建会话/二次 resume/取消回 planned/拒绝与 overview |
| web 契约 | `crates/web/tests/web_project.rs` + `web_project_runs.rs` | 3：三级 CRUD 契约、plan/execute 生命周期、503/409 形状 |
| SPA | `crates/web/spa/src/project/project.dom.test.jsx` | 6：tab/总览计数、markdown 详情、新建 modal、里程碑分组 PATCH、backlog 混排、抽屉取消 |

详见 [agents/project](../../../agents/project/index.md) / [store](../../../agents/store/index.md) / [web](../../../agents/web/index.md)。

## 回归门禁（2026-09-05，并发迭代环境）

- `cargo test -p opencoder-core -p opencoder-store -p opencoder-project -p opencoder-web --no-fail-fast`：107 suites ok / **852 passed**，仅 `core/tests/config_providers.rs` 13 例失败——归属：并发 sibling 的 provider 重构在测（env 泄漏断言 + PoisonError 级联，与 storage/project 无关）。
- store schema 已随 team v18 收敛：`store_migrations` 全绿（project 表 v15 块迁移兼容 `from < 15` 旧库路径）。
- `cargo clippy` 同四 crate `--all-targets -D warnings`：0 警告；`rustfmt --check`：我的文件 0 diff（仓内其余 diff 属 sibling 在改文件）。
- SPA：`check-spa-drift` no drift；vitest 全量 328/328（Phase 4 实跑）。
- 行数门禁：新文件 ≤400、迭代文件 ≤800 全过；敏感信息扫描 0 命中。
- sqlx 后端真容器验证：MySQL 8.4 / StarRocks 3.3 实测通过（`OC_TEST_MYSQL_DSN`/`OC_TEST_STARROCKS_DSN` 门控）。
