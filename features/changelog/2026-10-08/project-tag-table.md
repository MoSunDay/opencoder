Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# 项目中的独立 Tag 管理表格

项目工作台增加第四个 Tag 页签，统一展示标签定义及项目、专项归属。新建和编辑抽屉支持名称与归属修改，包含独立专项；项目和专项详情中的 Tag 管理入口移除。表格列筛选沿用项目工作台的视图状态，保存失败保留草稿，删除须确认。

Web 和 Server 共享 Tag 修改接口，提交完整名称、归属类型和归属 ID；原 Tag ID 保留。libsql 更新既有字段，无新增表或迁移。专项覆盖同名项目定义，已选 TODO 标签按保存后的名称重新匹配，无法匹配则解除；未选择标签的 TODO 不会自动添加标签。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 四个独立表格页签 | `has four table tabs and a global TODO table including unassigned work` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 同名定义分别展示、独立专项归属 | `keeps same-name definitions separate and resolves project and standalone owners` | [model.test.js](../../../crates/web/spa/src/project/tags/model.test.js) |
| 新建、改名和归属修改 | `lists both same-name definitions and creates a tag on a standalone initiative`、`clears the old owner on type change and saves name and scope using the original ID` | [tab.dom.test.jsx](../../../crates/web/spa/src/project/tags/tab.dom.test.jsx) |
| 保存失败保留草稿、防重复提交、筛选和删除确认 | `preserves the draft after failure and submits only once while a write is pending`、`keeps filters after saving and requires confirmation before deleting` | [tab.dom.test.jsx](../../../crates/web/spa/src/project/tags/tab.dom.test.jsx) |
| 名称和具体归属校验 | `rejects blank and overlong names and missing owners` | [tab.dom.test.jsx](../../../crates/web/spa/src/project/tags/tab.dom.test.jsx) |
| 移动保留 ID、清除范围外关联、不添加未选标签 | `moves::moving_project_tag_to_initiative_preserves_id_and_removes_out_of_scope_links` | [moves.rs](../../../crates/store/tests/project_tags/moves.rs) |
| 源项目回退、目标专项覆盖、独立专项与项目互转 | `moves::moving_local_override_restores_source_project_and_overrides_target_project` | [moves.rs](../../../crates/store/tests/project_tags/moves.rs) |
| 重名、无效归属原子拒绝及最终名称匹配 | `moves::moving_between_projects_rejects_duplicates_and_missing_owners_without_changes`、`moves::simultaneous_rename_and_move_matches_the_saved_name` | [moves.rs](../../../crates/store/tests/project_tags/moves.rs) |
| 专项优先与删除回退 | `local_override_remaps_existing_todos_and_delete_restores_project_tag` | [project_tags.rs](../../../crates/store/tests/project_tags.rs) |
| Web 实际路由的归属修改及错误拒绝 | `update_moves_tag_without_recreating_it_and_reconciles_todos`、`invalid_update_keeps_catalog_and_todo_links_unchanged` | [web_project_tag_moves.rs](../../../crates/web/tests/web_project_tag_moves.rs) |
| Server 实际路由的移动、重名拒绝和删除回退 | `project_tags::editable_tag_scope_works_on_server_and_rejects_conflicting_move` | [project_tags.rs](../../../crates/control/tests/e2e/project_tags.rs) |
| 四种屏宽、归属编辑、409 草稿与删除确认 | 浏览器脚本的完整断言 | [project_workbench_ui.js](../../../scripts/acceptance/project_workbench_ui.js) |

- 全量回归：`cargo test --offline --workspace --no-fail-fast` → **5720 passed / 0 failed / 8 ignored**；8 项为仓库原有手动测试，未新增跳过。原始日志 `/tmp/opencoder-tag-workspace-verified-20261008.log`，退出码和环境回执 `/tmp/opencoder-tag-workspace-verified-result-20261008.json`。
- Clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；日志 `/tmp/opencoder-tag-clippy-fixed-20261008.log`。
- 工作区构建、执行镜像所需示例构建、Rust 格式检查通过；日志 `/tmp/opencoder-tag-build-final-20261008.log`、`/tmp/opencoder-tag-examples-final-20261008.log`、`/tmp/opencoder-tag-format-final-20261008.log`。
- SPA 全量：149 个文件、1059 项测试通过；日志 `/tmp/opencoder-tag-spa-all-20261008.log`。SPA 构建与产物漂移检查通过。
- 全站 UI 与 TUI：16 项检查全部通过；回执 `/tmp/opencoder-tag-global-ui-20261008/receipt.json`。项目浏览器验收覆盖 1920、1280、768、390 四种宽度，页面错误为零；回执与截图 `/tmp/opencoder-tag-ui-20261008-final/`。全站响应式检查的进程预算调整为 600 秒，具体操作超时和断言保持不变。
- 发布脚本回归：63 项通过；日志 `/tmp/opencoder-tag-release-tests-20261008.log`。
- 全量测试使用独立空用户目录（位于 `/tmp` 之外）和允许设备访问的内存临时目录，构建缓存及版本匹配的测试镜像也使用本任务独立内存目录，避免登录脚本及磁盘读写影响现有超时断言；未修改执行逻辑或降低断言。

相关索引：[项目工作台](../../project/index.md)、[Web](../../../agents/web/index.md)、[Store](../../../agents/store/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。
