Commit: 529d076f8e1bb0808ba0b418ca114e945cb6f6f2

# 项目详情页签与 TODO 评论

项目详情分成概览、专项，专项详情分成概览、TODO 看板。TODO 详情分成任务、设置、执行、评论，默认阅读 Markdown 正文，点击编辑后修改并可预览。新建 TODO 使用任务、设置；项目和专项的概述及说明也共用 Markdown 编辑预览。

任务字段使用同一个 Ant Design Form，跨页签保留值；标题校验回到任务页，后台刷新和提交失败保留草稿。执行与评论仅在页签可见时读取，隐藏取消请求，迟到返回不会跳页；返回指派记录保留原生执行能力。

评论采用只追加的 Markdown 信息流。网页由当前用户署名，API/CLI 支持全部六类能力 ID，服务端保存名称/类型快照与实际提交用户。可选执行来源核验当前 TODO 关联及真实准入能力；相同用户、TODO 和 request_id 的重试返回原评论，相同键不同正文返回 409。默认最近 50 条，按递增序号读取更早记录与全部增量页。

schema 39 增加独立评论表、分页索引和 TODO 删除清理；不改写既有鉴权数据。追加更新任务时间，专项 summary-context 包含全部评论并报告失败来源。评论不改变看板状态，也不代替原生执行结果或任务验收。

评论接口需要配套的新 Server。数据库沿用既有 schema 版本校验，升级到 39 后不能直接交给只支持 38 的旧 Server；正式升级与回滚须遵循发布流程。本轮仅在独立临时数据库验证迁移。

## 测试覆盖

| 功能 | 用例 | 文件 |
| --- | --- | --- |
| 默认阅读、按需加载、预览、跨页签保存、隐藏校验及关闭草稿 | `opens Markdown reading...`、`preserves task edits...`、`asks before discarding...` | [todoTabs.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoTabs.dom.test.jsx) |
| 评论重试、署名、来源跳转、迟到读取与增量分页 | `keeps failed Markdown comments...`、`loads every incremental page...` | [panel.dom.test.jsx](../../../crates/web/spa/src/project/comments/panel.dom.test.jsx) |
| 执行隐藏取消、迟到读取不跳页、旧读取隔离及六类原生入口 | `keeps the task selected when dispatch reads...`、TODO 执行/派发相关用例 | [todoDrawer.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoDrawer.dom.test.jsx)、[todosTab.dom.test.jsx](../../../crates/web/spa/src/project/todosTab.dom.test.jsx) |
| 存储分页、并发幂等、来源事务核验、删除及旧版升级 | `pagination_has_no_gaps...`、`concurrent_retries_are_idempotent...`、`version_38_upgrade_preserves_tasks_and_authentication_rows` | [存储测试](../../../crates/store/tests/project_comments.rs) |
| 认证用户、权限、输入校验、六类署名及来源准入 | `comments_use_authenticated_users...`、`all_six_capability_signatures...`、`comment_execution_sources_match_admission...` | [HTTP 测试](../../../crates/control/tests/e2e/project_api/comments.rs) |
| CLI 解析、JSON 文件、分页及概述读取全部评论 | `comment_commands_parse_pagination...`、`initiative_reads_all_assignments_pages_full_messages...` | [CLI 测试](../../../crates/ctl/tests/project_comments.rs)、[概述测试](../../../crates/ctl/tests/project_summary.rs) |

## 定向验证

- SPA：最后一次核心回归 9 个文件、39 个用例通过；直接相关的 Markdown、概述、人员、派发和结果组件另有 25 个用例通过，共 64 个不同用例。未运行 SPA 全量测试。
- Rust：评论存储 4、分页规则 1、评论 HTTP 3、CLI 1、概述 5、原生执行关联 8，共 22 个不同用例通过；迁移另核验临时数据库中的既有用户和 Token 摘要保持有效。
- `cargo clippy -p opencoder-store -p opencoder-control -p opencoder-cli --all-targets -- -D warnings` 通过；迁移测试修改后的定向 Clippy 也通过。Rust 格式与 `git diff --check` 通过。
- SPA、Server、CLI 构建通过，独立 Server 内嵌 SPA 与测试产物摘要一致：`d3e803e55f5c8ffac65de02b0ce4e7c26f79c11e7b898461e459779f81559d1b`。
- [真实交互验收](../../../scripts/acceptance/project/forms.js) 使用独立 Server、CLI 和临时数据库，44 组测量覆盖 1920、1280、768、390 四种屏宽，验证真实创建、编辑、失败重试、评论追加、分页、关闭草稿和详情页签；浏览器异常为 0。
- [排序验收](../../../scripts/acceptance/project_order_ui.js) 使用独立 Server，验证概述历史、筛选拖拽、失败回滚、刷新持久化和键盘排序；四种屏宽通过，浏览器异常为 0。
- [工作台验收](../../../scripts/acceptance/project_workbench_ui.js) 使用可变 API 夹具，验证四种屏宽、详情导航、筛选拖拽、Tag 分组同步、失败回滚和 Tag 归属切换；全部通过，浏览器异常为 0。真实持久化由上面的独立 Server 场景与 Rust 测试取证。
- 新增文件不超过 400 行，新增逻辑所在目录不超过 10 个逻辑文件。仅更新代码和本地产物，未部署生产。

相关说明：[项目工作台](../../project/index.md)、[项目模块](../../../agents/project/index.md)、[项目规则](../../../rules/07-project-module-contract.md)、[验收入口](../../../scripts/acceptance/project/README.md)。
