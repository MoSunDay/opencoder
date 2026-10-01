Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 项目表格、专项看板抽屉与 Tag 管理

项目工作台按项目、专项、TODO 三个页签组织。项目和专项使用表格与右侧详情抽屉，专项内管理 TODO 看板；项目进度按所有专项的实际 TODO 数量计算。所有数据列支持筛选，保存刷新和关闭抽屉后保留视图条件。窄屏表格在内部横向滚动。

项目与专项可维护 Tag，一个 TODO 可有多个 Tag；专项覆盖同名项目 Tag。分组卡片共享一个 TODO，拖动更新状态与完整顺序，保留筛选隐藏的卡片，写入失败回退。存储拒绝外部范围的 TODO 和无效 Tag 选择，并在支持事务的后端原子提交。

schema v32 使用专项表与 Tag 定义、关联表，移除项目里程碑接口和旧容器结构。升级校验专项复制结果，旧里程碑下的 TODO 解除归属，历史执行记录保留。SQLite、MySQL、StarRocks 的存储实现及 Web、Control、CLI 的项目接口同步调整；没有运行生产数据库迁移。

历史升级初始化按已有版本识别旧结构，不依赖会话表存在。v9 测试数据移除当时尚不存在的项目表；v31 测试使用历史表名、列名与索引，并验证连续打开后 TODO 归属与内容。

回归中修复画布编辑器依赖连线显示但未保存的问题；拆分触及的长测试文件，并补齐新协议测试夹具中的范围与能力声明。DAG 执行方案仍受[执行约定](../../../rules/04-dag-execution-contract.md)约束。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 三个表格与未归属 TODO | `has exactly three tabs and a global TODO table including unassigned work` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 项目进度与专项抽屉 | `opens project progress, then the initiative board in a right-side drawer` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 保存后筛选保留 | `filters a table by its title column and preserves the filter after a refresh` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 加权进度与 Tag 筛选延续 | `weights project completion by the actual TODO count`、`retains tag filters on rename and same-name scope changes` | [catalog.test.js](../../../crates/web/spa/src/project/model/catalog.test.js) |
| 文本、Tag、数值、日期列筛选 | `filters text, multiple tags, inclusive numeric ranges and complete date days` | [table.test.jsx](../../../crates/web/spa/src/project/model/table.test.jsx) |
| 同名覆盖与删除回退 | `local_override_remaps_existing_todos_and_delete_restores_project_tag` | [project_tags.rs](../../../crates/store/tests/project_tags.rs) |
| Tag 归属与原子拒绝 | `tag_scope_rejects_missing_owner_and_cannot_move_existing_definition`、`invalid_selection_rolls_back_todo_creation_and_edits` | [project_tags.rs](../../../crates/store/tests/project_tags.rs) |
| 旧结构迁移与重复打开 | `v31_migration_removes_legacy_containers_but_preserves_todo_payloads` | [project_tags.rs](../../../crates/store/tests/project_tags.rs) |
| 部分旧表升级与回放、备份保留 | `schema_migration_v19_to_v20_adds_project_executor_columns`、`v20_backup_preserves_history_and_v21_replay_survives_reopen` | [catalog.rs](../../../crates/store/tests/store_migrations/catalog.rs)、[project_replay.rs](../../../crates/store/tests/store_migrations/project_replay.rs) |
| v9 输入记录回填 | `migration_v9_to_v10_backfills_recorded_for_promoted_rows` | [suite_1.rs](../../../crates/store/tests/inputs_recorded/suite_1.rs) |
| 项目删除保留数据 | `delete_goal_preserves_initiative_todo_and_runs` | [suite_2.rs](../../../crates/store/tests/project_store/suite_2.rs) |
| HTTP Tag 增删改与错误状态 | `tag_list_rename_and_delete_validate_scope_and_missing_records`、`tags_validate_scope_override_names_and_round_trip_in_overview` | [web_project_tags.rs](../../../crates/web/tests/web_project_tags.rs) |
| 范围内拖动与看板进度 | `scoped_reorder_rejects_foreign_cards_and_progress_uses_board_status` | [web_project_tags.rs](../../../crates/web/tests/web_project_tags.rs) |
| 悬空归属与历史分页 | `overview_full_projection_contract`、`runs_pagination_cursor_contract` | [web_project_mock_dataset.rs](../../../crates/web/tests/web_project_mock_dataset.rs) |
| CLI 专项接口 | `project_initiatives_filter_and_crud` | [parse_project_brain_agents.rs](../../../crates/ctl/tests/parse_project_brain_agents.rs) |
| 画布连线保存 | `连线模式点击两个步骤建立依赖并保存` | [editor.dom.test.jsx](../../../crates/web/spa/src/dag/editor/editor.dom.test.jsx) |
| 浏览器拖动、分组同步、失败回退与 CRUD | `project_workbench_ui.js` 的全部断言 | [验收脚本](../../../scripts/acceptance/project_workbench_ui.js) |

- SPA 全量回归：116 个文件、908 项测试通过，日志 `/tmp/opencoder-project-spa-percent-final.log`。
- 项目存储与 HTTP 专项：运行 Cargo 构建的 `project_tags`、`project_relations`、`project_store`、`web_project_tags`、`web_project_mock_dataset`、`web_project` 六个测试程序，37 项通过，日志 `/tmp/opencoder-project-current-scoped-tests.log`。
- 修复历史升级后，`cargo test -p opencoder-store --no-fail-fast -j 8` 全部通过：303 项测试、0 失败、0 忽略；日志 `/tmp/opencoder-project-store-final.log`。
- Control 的 `project_api`、`project_crud_extra`、`project_store_failure` 共 24 项用例在工作区全量回归中通过；日志 `/tmp/opencoder-project-workspace-tests-current.log`。项目归档用例超时后单独重跑通过，日志 `/tmp/opencoder-project-archive-rerun.log`。
- 浏览器验收：1920、1280、768、390 像素宽度通过；筛选后完整顺序、Tag 分组同步、保存失败回退、同名范围覆盖、Tag 与项目 CRUD 均通过，页面错误为零。回执与截图在 `/tmp/opencoder-project-ui`。
- SPA 构建通过，产物从临时目录逐文件替换，避免 Cargo 编译期间丢失内嵌资源。日志 `/tmp/opencoder-project-spa-percent-build.log`。
- MySQL/StarRocks 可选后端：历史升级修复后，`cargo clippy -p opencoder-store --all-targets --features mysql,starrocks -- -D warnings` 通过；本次没有配置可选 SQL 实例，未做真实数据库验收。日志 `/tmp/opencoder-project-sql-clippy-latest.log`。
- 历史升级修复后，工作区构建通过，日志 `/tmp/opencoder-project-workspace-build-latest.log`。共享工作区的 DAG 迁移仍在修改；最新全量测试及 Clippy 未通过，不能视为完整回归通过。
- 修复前全量测试使用 `cargo test --workspace --no-fail-fast -j 8`，5536 项通过、67 项失败，30 个目标失败；日志 `/tmp/opencoder-project-workspace-tests-current.log`。其中存储升级问题已由上面的 303 项存储回归确认修复。其余失败包含 DAG 根文件系统未配置、容器执行断言和共享代码变化造成的编译问题。
- 最新 `cargo test --workspace -j 8` 因 `crates/worker/examples/resource_snapshot.rs` 引用已移除的 `src/resources/snapshot.rs` 而停止；日志 `/tmp/opencoder-project-workspace-tests-latest.log`。
- 最新 Clippy 失败来自 DAG 测试夹具仍传入已移除的 `ExecDeps.client`，日志 `/tmp/opencoder-project-workspace-clippy-latest.log`。

相关索引：[项目工作台](../../project/index.md)、[项目逻辑](../../../agents/project/index.md)、[存储逻辑](../../../agents/store/index.md)、[Web 逻辑](../../../agents/web/index.md)。
