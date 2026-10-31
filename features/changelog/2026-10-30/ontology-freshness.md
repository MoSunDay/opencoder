# Ontology 数据保鲜：级别阶梯、复核运行与人工评审

一句话：实体新增四级数据级别 `data_level`（draft/stable/verified/confident，旧实体默认 draft 零迁移），按实体类型配置保鲜策略（agent/DAG 执行器 + 并发 + 可选回写属性），控制面触发运行快照至多 500 个实体并有界扇出执行，人工评审通过后级别恰好提升一级（confident 封顶）并可回写不可变 Markdown 正文版本；策略、运行、评审落三张 JSON 正文表，存量库经标准 `--maintenance` 维护窗口补表。

## 变更清单

- 新增 `crates/ontology/src/domain/freshness.rs` — 纯域：级别阶梯、策略/运行/评审类型、校验与结算规则（并发 1..30、运行项 ≤500、候选 ≤8000 字符）；`crates/ontology/src/domain/types.rs` 实体 body 增加 `data_level`（serde 默认 draft）。
- 新增 `crates/ontology/src/database/freshness.rs`、`database/freshness_review.rs` — `freshness_policies`/`freshness_runs`/`freshness_reviews` 三张 JSON 正文表读写、运行快照与单事务评审（重复 409、目录托管保护、回写正文版本、级别 +1 与实体 revision 递增）。
- `crates/ontology/src/database/mod.rs`、`schema/tests.rs`、`seeding.rs`、`entities.rs` — 表接入、结构维护清单（legacy 清单纳入三表）与实体快照查询；schema 版本保持 1。
- 新增 `crates/ontology/src/api/freshness.rs` 并接入 `api/mod.rs` — `/api/ontology/envs/:env/freshness/*` 数据平面路由（policies 读写、runs 读、reviews 读写；写入 `require_manage`）。
- 新增 `crates/ontology/src/freshness.rs`（`AppState::freshness` 操作面）及 `lib.rs`、`domain/mod.rs` 导出；运行仅经此入口创建与更新。
- 新增 `crates/control/src/ontology_freshness/`（`api.rs` 触发路由、`orchestrator.rs` 派发与轮询、`planning.rs` 纯函数、`reconcile.rs` 重启回收）并接入 `routes.rs`、`lib.rs`；`api/executions/results/mod.rs` 提供执行结果读取。
- 新增 `crates/ctl/src/cmd/ontology/freshness.rs` 并接入 `cmd/ontology/mod.rs` — `opencoder-cli ontology freshness` 的 `policies list/get/create/update`、`runs list/get/trigger`、`reviews list/create`。
- `scripts/platform/rolling/native/ontology_schema.sql`、`rolling_tests/ontology/test_schema.py` — 维护窗口补齐三表；`scripts/acceptance/release_compatibility/schema.py` 增补 `freshness_policies` 断言。
- SPA：新增 `crates/web/spa/src/ontology/pages/FreshnessPage.tsx` 与 `pages/freshness/`（PolicyTable、RunsTable、ReviewQueue、LevelBadge、levels）；`panels.tsx`、`nav.js`、`shell/panels.jsx` 注册「数据保鲜」页；`EntityDetailDrawer.tsx` 等展示级别徽标。
- 文档：`rules/10-ontology-cli.md` 命令表、`features/ontology/index.md`、`agents/ontology/index.md`、`agents/web/ontology.md`、`repo-memory.md`、`features/index.md`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 级别解析、排序与单步推进；serde snake_case | `data_levels_parse_order_and_advance`、`data_level_serde_uses_snake_case_names` | `crates/ontology/src/domain/freshness.rs` |
| 策略校验（空名、越界并发、执行器提示词规则） | `policy_validation_rejects_blank_names_and_bad_concurrency`、`executors_follow_kind_specific_prompt_rules` | `crates/ontology/src/domain/freshness.rs` |
| 结构维护清单纳入三张保鲜表 | `legacy` | `crates/ontology/src/database/schema/tests.rs` |
| 策略 CRUD 校验与 revision | `policy_crud_enforces_validation_and_revisions` | `crates/ontology/tests/freshness.rs` |
| 运行生命周期收敛与 scope 锁定 | `run_lifecycle_settles_status_and_locks_scope` | `crates/ontology/tests/freshness.rs` |
| 评审通过/拒绝与候选回写 | `reviews_approve_reject_and_write_back_candidates` | `crates/ontology/tests/freshness.rs` |
| viewer 只读保鲜数据 | `viewer_role_reads_freshness_but_cannot_mutate` | `crates/ontology/tests/freshness.rs` |
| 无节点时触发运行逐项优雅收敛 | `triggering_a_run_settles_items_gracefully_without_nodes` | `crates/control/tests/ontology_freshness.rs` |
| 未知策略触发在建运行前失败 | `triggering_with_an_unknown_policy_fails_before_creating_a_run` | `crates/control/tests/ontology_freshness.rs` |
| 执行 ID 确定性/提示词渲染/输入冻结/并发与目标消毒/结算与状态机/进度合并/轮询读取映射 | `execution_ids_are_deterministic_distinct_and_fleet_legal`、`render_prompt_substitutes_known_and_blanks_unknown_fields`、`execution_input_freezes_agent_and_dag_shapes`、`concurrency_and_target_are_sanitized`、`settle_matches_the_data_plane_rule`、`transitions_allow_only_queued_start_and_running_finish`、`merge_progress_moves_items_forward_only_by_entity_id`、`poll_and_read_outcomes_map_statuses_and_bounds` | `crates/control/src/ontology_freshness/planning_tests.rs` |
| CLI freshness 命令到端点映射（9 项） | `parse_ontology` freshness 用例 | `crates/ctl/tests/parse_ontology.rs` |
| 维护窗口补齐保鲜三表 | `rolling_tests.ontology.test_schema` 各用例 | `scripts/platform/rolling_tests/ontology/test_schema.py` |
| 级别阶梯中文名、配色与封顶推进 | `levels.test.ts` 各用例 | `crates/web/spa/src/ontology/pages/freshness/levels.test.ts` |
| 页面三 tab、运行状态与评审队列 | `FreshnessPage.dom.test.tsx` 各用例 | `crates/web/spa/src/ontology/pages/FreshnessPage.dom.test.tsx` |

- 定向回归：`cargo test -p opencoder-ontology` → 65 passed / 0 failed；`cargo test -p opencoder-control --test ontology_freshness` → 2 passed / 0 failed；`cargo test -p opencoder-control --lib ontology_freshness` → 8 passed / 0 failed；`cargo test -p opencoder-cli --test parse_ontology` → 全部通过（含 freshness 9 项映射）。
- 维护回归：`cd scripts/platform && /usr/bin/python3 -m unittest rolling_tests.ontology.test_schema` → 5 passed / 0 failed；`scripts/acceptance/release_compatibility/schema.py` 已扩展 `freshness_policies` 断言，需构建发布产物后执行。
- SPA（`crates/web/spa`）：`npm run typecheck:ontology` → 零错误；vitest `src/ontology/pages/freshness/levels.test.ts`、`src/ontology/pages/FreshnessPage.dom.test.tsx` → 通过；`bash scripts/check-spa-drift.sh` → 无漂移。
- Clippy：`cargo clippy -p opencoder-ontology -p opencoder-control -p opencoder-cli --all-targets` → 零警告。
- 测试范围：本次改动及其直接影响链路；未运行全量测试。

## 决策记录

- 三张保鲜表放进 Ontology 数据库并走标准 `--maintenance` 滚动窗口，而非存到控制面：评审需要与实体、正文版本在同一事务里结算（重复 409、回写正文、级别 +1、revision 与审计原子完成），跨库无法保证一致性；schema 版本保持 1，仅补表。
- 触发路由放在控制面（`POST /api/ontology/envs/:env/freshness/policies/:id/runs`，202）：运行写入保持单写者，数据平面提供策略/运行/评审读写但不驱动运行。
- `data_level` 作为实体 body 的 JSON 字段（serde 默认 draft），不加表头或列：旧记录零迁移即可读取为 draft。
- 控制面结算复用数据平面 `settle` 规则（succeeded/partial/failed），单一定义避免两端分叉。
- Phase 2 边界（本轮明确不做）：定时调度、运行取消、模型预审、属性粒度级别、图谱子集圈定。

## 登记核对

- Ontology 能力目录描述已扩展（加入「数据保鲜」），`/api/ontology` 前缀此前已登记，无新增前缀；能力同步测试无需新增。
- 无新增 CLI 域：`freshness` 挂在 `opencoder-cli ontology` 下，`rules/10-ontology-cli.md` 命令表已含对应行。
