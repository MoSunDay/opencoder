Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# 大脑画布编辑回退修复与项目 Tag 菜单

画布原先同时维护计划与 React Flow 节点副本，键盘删除只改变显示副本，后续计划重绘会恢复节点；部分编辑回调还会用旧草稿覆盖较新的字段和布局。

计划现在统一决定层级和节点内容，显示状态仅保存选择、测量和拖动过程；所有编辑动作合并到最新草稿。按钮与 Backspace/Delete 共用删除确认，支持多选，删除层时同时清理其子节点。删除或移动子节点保留各层手动位置，迟到的位置事件不能恢复已删层；输入框退格和只读预览不触发删除。关闭重开保留最新浏览器草稿及布局，保存版本提交最新计划内容；空层草稿在提交前仍须补齐节点。

项目分类下原“项目”菜单更名为 Overview，保留项目、专项、TODO 三个页签；Tag 独立为同级菜单，复用现有标签表格、权限与 API，读取失败可重试。桌面与移动导航均覆盖两个入口。

全量回归同时更新了两处 Operator 测试的严格提示词预期，验证每轮默认规则与原用户消息；节点 smoke 在节点在线、空闲且能力快照就绪后才派发任务。未修改后端协议、表结构或环境变量。

相关说明：[大脑工作台](../../brain/index.md)、[项目工作台](../../project/index.md)、[Web 逻辑](../../../agents/web/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 连续编辑保留字段、绑定、层级与位置 | `successive edits preserve fields, capability bindings, membership and unrelated positions` | [editing.test.js](../../../crates/web/spa/src/brain/workbench/tests/editing.test.js) |
| 批量级联删除与迟到位置更新 | `batch deletion cascades once and late position events cannot restore deleted layers` | [editing.test.js](../../../crates/web/spa/src/brain/workbench/tests/editing.test.js) |
| 布局增量合并、显式整理及节点上限 | `merges separate position updates and only an explicit reset clears layout`、`enforces layer and parallel node limits against the current draft` | [editing.test.js](../../../crates/web/spa/src/brain/workbench/tests/editing.test.js) |
| 旧回调与立即关闭保存最新草稿 | `queued callbacks and an immediate close persist the latest complete draft`、`rejected edits leave the current draft and storage unchanged` | [draft.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/tests/draft.dom.test.jsx) |
| 重绘保留拖动、测量及最新任务 | `a plan redraw preserves an active drag and measurements while showing the latest task` | [presentation.test.js](../../../crates/web/spa/src/brain/workbench/milestone/canvas/presentation.test.js) |
| 位置提交与只读显示边界 | `drag completion and keyboard movement commit positions but selection and measurements do not`、`display events never change plan membership and readonly position events are ignored` | [presentation.test.js](../../../crates/web/spa/src/brain/workbench/milestone/canvas/presentation.test.js) |
| Backspace/Delete 删除后继续编辑、重开及保存 | `%s 删除同层节点后继续编辑、重开和保存均不恢复节点`（两种按键） | [editor.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/milestone/canvas/editor.dom.test.jsx) |
| 删除确认、输入框与只读保护 | `取消键盘删除和输入框退格不修改计划`、`历史预览的键盘不能删除节点` | [editor.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/milestone/canvas/editor.dom.test.jsx) |
| 层级删除范围与空层提交校验 | `按钮删除子节点保留各层坐标，删除整层仅清理该层，整理布局才清空坐标`、`删除层内最后一个节点保留空层草稿，提交时要求补齐节点` | [editor.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/milestone/canvas/editor.dom.test.jsx) |
| Overview 与独立 Tag 导航 | `scopes the project category to Overview and the standalone Tag page` | [appNavigation.dom.test.jsx](../../../crates/web/spa/src/appNavigation.dom.test.jsx) |
| Tag 保存后刷新、读取错误重试与只读权限 | `opens the standalone Tag table and reloads it after saving`、`failed initial loading shows the error and can retry instead of showing an empty table`、`viewer can open Tag but cannot create, edit or delete definitions` | [panel.dom.test.jsx](../../../crates/web/spa/src/project/tags/panel.dom.test.jsx) |
| 真实浏览器画布编辑与菜单操作 | `brain-editor`、`project-board` | [editor.js](../../../scripts/acceptance/brain/ui/editor.js)、[project_workbench_ui.js](../../../scripts/acceptance/project_workbench_ui.js) |
| Operator 恢复保留默认规则、用户消息与线程 | `operator_recovery_completes_each_initialization_stage_and_keeps_server_profile`、`registered_codex_operator_uses_server_settings_auto_placement_and_same_thread` | [operator_initialization_recovery.rs](../../../crates/worker/tests/operator_initialization_recovery.rs)、[tui_operator.rs](../../../crates/worker/tests/tui_operator.rs) |
| 节点就绪后接受任务 | `CK1` 及后续任务执行检查 | [smoke_nodes.sh](../../../scripts/smoke_nodes.sh) |

## 验证结果

验证基于上述提交的工作区固定快照；并行开发期间隔离源码及测试数据，最终比对 SPA 源码与构建产物均与当前工作区一致。原生集成和连续性验收使用私有临时内存文件系统；结果属于开发验收，不是生产部署或磁盘性能结论。SPA 产物摘要为 `9554374a213d8736be5d8ab8899484ee85029d4ea3395e84002bf5460a9ac8ca`。

证据目录：`/tmp/opencoder-canvas-tags.2rpZ0Z/`。

- SPA：166 个测试文件、1,157 个用例通过；构建及产物漂移检查通过。日志：`spa-tests-current.log`，源码核对：`spa-final-source-comparison.json`。
- 全量 Rust：`cargo test --workspace --no-fail-fast`，5,770 passed / 0 failed / 8 既有 ignored。实际输出：`native-verified-tests.log`，汇总：`native-verified-summary.json`。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace` 均通过。日志：`native-final-fmt.log`、`native-final-clippy.log`、`native-verified-build.log`。
- 发布脚本五组回归共 171 项通过，UI 验收工具测试 6 项通过。回执：`release-scripts.json`、`ui-harness-tests.log`。
- 全站浏览器及 TUI 验收 17 项检查通过，覆盖 17 个页面及 1920、1280、768、390 四种宽度；无页面外溢或浏览器错误。回执：`global-ui/receipt.json`，功能截图与记录：`global-ui/native-artifacts/`。
- 独立升级与回滚验收通过：原有 DAG、TODO、shell 与 OCI 容器执行连续，持续提交 131 次；回滚后观察 900.75 秒，58 个观察任务全部完成。回执：`smooth-memory-evidence/opencoder-smooth-0uqe0ckg/result.json`、`observation.json`。测试服务已退出。
