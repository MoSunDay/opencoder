Commit: 35a3fd069d22848ce0c27d48c6c3f222ccb97f12

# 项目 TODO 看板与能力指派

## 背景

项目工作台按工作进度组织 TODO，并让 TODO 指派原生能力执行、回看结论。

## 变更

- React + Ant Design 项目页改为四列 TODO 看板，使用 dnd-kit 拖动并持久化列状态与顺序。
- TODO 改为通过“指派 Agent/能力”按钮进入 Agent、Team、DAG、TODO 工作流或 Brain 原生界面；标题与说明预填为可编辑执行输入，不再解析或自动派发 `@` 标记。
- 沿用 TODO–执行关联表保存每次指派的类型、名称、执行 ID、状态与独立结论；后台读取执行产物回写结论，最新一次有非空结论才显示完成，不覆盖 TODO 计划。
- 右侧抽屉展示指派历史和原生执行详情；Team 运行中可提交引导。看板展示最新指派是否已回写结论，人工看板列不随执行自动改变。
- SQLite schema 升至 v31，MySQL/StarRocks 同步补齐指派字段；既有执行关联原样保留。旧项目执行 API 仍供历史回放使用。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 看板拖动计算 | `moves a card across lanes and orders the entire destination lane` | `crates/web/spa/src/project/model/board.test.js` |
| TODO 排序与回滚 | `board_reorder_moves_once_and_rolls_back_on_unknown_id` | `crates/store/tests/project_store.rs` |
| Web 排序接口 | `todo_board_reorder_moves_cards_and_rejects_missing_ids` | `crates/web/tests/web_project.rs` |
| TODO 不解析 @ 文本 | `todo_draft_does_not_dispatch_mentions` | `crates/web/tests/web_project.rs` |
| 五类能力关联与幂等 | `all_five_native_capabilities_can_be_assigned`、`assignment_link_is_idempotent_and_records_kind_name_and_result` | `crates/control/tests/e2e/project_links.rs` |
| 结论与旧关联迁移 | `todo_assignments_record_result_and_cascade_on_delete`、`v30_execution_links_upgrade_without_losing_ids` | `crates/store/tests/project_relations.rs` |
| 原生界面输入与指派记录 | `includes editable TODO context in the native DAG dispatch`、`submits editable TODO context through the native workflow run`、`submits Team guidance with a stable input ID from the execution detail` | `crates/web/spa/src` |
| 旧项目执行总览 | `project_plan_act_and_new_draft_stay_on_the_assigned_node` | `crates/worker/tests/platform/workloads.rs` |

- 定向 Rust 测试：`project_relations`、`project_links`、`web_project` 通过。
- 前端构建、定向前端测试 56/56、全量前端测试 900/900、定向 Rust 测试 14/14 通过；MySQL/StarRocks 特性编译检查通过。
- 独立浏览器验收通过：创建项目、里程碑、关联和独立专项、各类 TODO；从专项 TODO 发起 Agent、从独立 TODO 发起 Operator，均验证执行 ID、结论内容 `fixture completed`、指派终态及无浏览器错误；390px 视口截图确认抽屉完整显示。
- `cargo fmt --all -- --check` 与 `git diff --check` 通过；全量回归与 clippy 见 [2026-09-29 收口记录](../2026-09-29/release-gate-closure.md)。

## 验收说明

浏览器验收提供 `--workbench-only` 入口，可独立验证项目层级、Agent 指派、执行 ID 与结论回写；不依赖旧项目执行回放的取消场景。

相关现状：[项目工作台](../../project/index.md)、[Web 模块](../../../agents/web/index.md)、[Store 模块](../../../agents/store/index.md)。
