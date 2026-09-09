Commit: (working-tree, 基于 56e612b6f251e3ed384074c7cf9cfc70a8c4c9cf)

# 独立专项里程碑、快速关联与编辑内容保留

## 背景与行为

项目 Markdown 在后台轮询或切换预览时会丢失输入。三级依赖也让专项与临时 TODO 的创建、查找和关联需要反复进入父级。

- 项目及里程碑 Markdown 按打开会话和记录 ID 初始化，预览读取同一表单；TODO 草稿保留本地缓冲，不随执行状态更新时间重置。
- Prompt、Env、Harness、DAG 等编辑器不因通知回调或同记录的新对象重载内容。资源切换隔离会话，保存期间禁用编辑，读取失败禁止用默认空值覆盖。
- 里程碑承担独立专项职责，可不关联项目；TODO 可不关联里程碑，项目归属从里程碑派生。不增加 Topic 表或重复项目字段。
- 里程碑与 TODO 改为全量列表，支持名称/状态/归属筛选、搜索名称或 ID 的 Select 关联及清空。项目和里程碑提供下一级列表跳转；进展与 Owner 视角统计包含独立专项。
- 删除项目解除里程碑归属，保留 TODO 和历史；非空里程碑删除返回 409，需先移动或解除 TODO 关联。

## 迁移与兼容

libsql schema v23 保留里程碑所有字段并放宽 `goal_id`。升级时将历史 backlog 一次性归入独立“待归类”，后续新建或解除关联的 TODO 保持未分组。MySQL/StarRocks 以列注释记录迁移进度，不增加表或环境变量；StarRocks 等待异步 schema 发布，失败明确报错并保留重试检查点。

已有接口字段名保留。里程碑 PATCH 的 `goal_id` 省略表示不修改、`null` 表示解除、ID 表示关联；总览新增 `standalone_milestones`，既有 `goals` 和 `backlog` 保留。无项目的里程碑可完成 Plan/Execute，执行上下文省略项目段落。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Markdown 同记录刷新、预览及失败保存 | `keeps all fields and preview mode when the same record receives a new object`、`saves from preview and retains every field after a failed save` | [editing.dom.test.jsx](../../../crates/web/spa/src/project/views/editing.dom.test.jsx) |
| 3 秒/8 秒轮询、TODO 保存缓冲和旧响应隔离 | `survives actual %s overview polling`、`failed TODO saves keep the draft; an old save cannot overwrite a different TODO` | [editing.dom.test.jsx](../../../crates/web/spa/src/project/views/editing.dom.test.jsx) |
| 独立创建、搜索/清空关联及列表跳转 | `creates a standalone milestone without any project and navigates its TODO list`、`searches associations by label or ID, sends a single ID and clears explicitly` | [relations.dom.test.jsx](../../../crates/web/spa/src/project/views/relations.dom.test.jsx) |
| Prompt/Harness/DAG 内容保留与读取错误 | `Prompt read errors cannot become an empty overwrite`、`Harness notification rerenders preserve fields and selected profile`、`DAG JSON edits survive a fresh object for the same definition` | [editors.dom.test.jsx](../../../crates/web/spa/src/ui/editing/editors.dom.test.jsx) |
| Env 草稿、变量、失败保存和读取失败保护 | `preserves text and variables through rerenders and a failed save`、`blocks saving when the initial read fails` | [envsPanel.dom.test.jsx](../../../crates/web/spa/src/envsPanel.dom.test.jsx) |
| libsql 可空关系及历史数据只迁移一次 | `standalone_milestone_and_optional_todo_association_roundtrip`、`v22_upgrade_preserves_data_and_classifies_only_legacy_backlog_once` | [project_relations.rs](../../../crates/store/tests/project_relations.rs) |
| MySQL/StarRocks 实际升级与删除保留 | `mysql_relations_upgrade`、`starrocks_relations_upgrade` | [sql_relations.rs](../../../crates/store/tests/sql_relations.rs) |
| 删除项目保留 TODO/运行，非空里程碑拒删 | `delete_goal_preserves_milestone_todo_and_runs`、`delete_milestone_requires_explicit_unlink_and_preserves_runs` | [project_store.rs](../../../crates/store/tests/project_store.rs) |
| Web 和 Control 的接口契约 | `standalone_relations_and_protected_deletion`、`standalone_milestones_appear_in_overview_and_guard_todos` | [web_project.rs](../../../crates/web/tests/web_project.rs)、[project_crud_extra.rs](../../../crates/control/tests/e2e/project_crud_extra.rs) |
| 独立专项完整 Plan/Execute | `standalone_milestone_can_plan_and_execute_without_a_project` | [plan_and_execute.rs](../../../crates/project/tests/plan_and_execute.rs) |

## 验证证据

- SPA 最终全量 496 项通过；重点回归 67 项、Agent/Env 11 项通过。SPA 构建成功，保留既有 bundle 体积提示。
- Store 默认全量 235 项、Project 全量 55 项、Control 项目接口 24 项及 Web 项目接口 4 项通过。
- MySQL 和 StarRocks 使用每次新建的隔离数据库完成实际升级测试，2 项通过；没有修改现有业务或鉴权数据。SQL feature 全目标 Clippy 零警告。
- Chromium 直接加载打包页面，7 项检查通过：真实 8 秒轮询保留 Markdown、预览保存、搜索 ID 关联、清空关联、独立专项创建、关联 TODO 跳转、TODO 草稿跨轮询保留；页面异常为 0。API 为隔离夹具，后端行为由上述真实数据库和 HTTP 测试验证。
- `cargo clippy --workspace --all-targets -- -D warnings`：全仓零警告，见 `/tmp/opencoder-relations-workspace-clippy.log`。
- `cargo test --workspace -j 8 --no-fail-fast -- --test-threads=1`：360 个 suite，**4951 passed / 0 failed / 5 ignored**，见 `/tmp/opencoder-relations-workspace-tests.log`。5 项既有手动用例为 2 项 NFS 挂载/离线测试及 3 项 runc/rootfs 测试；未新增 ignore。
- `cargo build --workspace -j 8`：成功，见 `/tmp/opencoder-relations-workspace-build.log`；新增文件行数与 `git diff --check` 通过。
- 证据日志位于 `/tmp/opencoder-relations-*.log`，浏览器报告为 `/tmp/opencoder-relations-browser.json`。

## 相关文档

[项目逻辑](../../../agents/project/index.md)、[存储](../../../agents/store/index.md)、[Web](../../../agents/web/index.md)、[平台能力](../../agent-platform/index.md)。
