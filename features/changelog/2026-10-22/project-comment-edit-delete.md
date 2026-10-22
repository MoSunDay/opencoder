Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# TODO 评论编辑与删除

TODO 评论可在原条目上编辑正文或确认删除。编辑保留编号、署名、执行来源和创建时间；editor 管理自己提交的评论，admin 管理全部评论，viewer 只读。API、CLI 和网页使用相同的权限与版本检查。

读取增加 `revision`、`updated_at`。PATCH 正文和 DELETE 查询均须提供 `expected_revision`；版本过期返回 409，前端保留编辑草稿。重复删除同一版本成功，原追加请求不能恢复已删除评论。原追加请求在编辑后重试返回当前正文。

状态表按需保存原追加内容、版本和删除标记，初始化不改写已有任务或认证数据。分页的 `after_seq` 仍只表示新增序号；网页刷新已加载范围，才能同步原条目的编辑与删除。操作不修改 TODO 看板，也不修改执行记录。

## 测试覆盖

| 功能 | 测试或入口 | 文件 |
| --- | --- | --- |
| 编辑保留身份、原请求重试、并发冲突、删除及权限 | 5 项评论修改用例 | [project_comment_mutations.rs](../../../crates/store/tests/project_comment_mutations.rs) |
| 既有追加、分页、来源和旧库初始化 | 4 项原评论用例 | [project_comments.rs](../../../crates/store/tests/project_comments.rs) |
| 真实 HTTP 权限、输入限制、冲突、删除及六类署名 | `project_comment` 过滤的 4 项 | [HTTP 用例](../../../crates/control/tests/e2e/project_api/comment_mutations.rs) |
| CLI 解析、请求分发和上下文收集 | 11 项评论、context、summary 用例 | [CLI 用例](../../../crates/ctl/tests/project_comments.rs) |
| 编辑、确认删除、失败草稿、刷新和 TODO 页签 | 20 项相关 DOM 用例；失败项修正后定向复测通过 | [评论面板](../../../crates/web/spa/src/project/comments/panel.dom.test.jsx) |
| 真 Server、浏览器、并发修改和四档宽度 | 独立验收脚本 | [评论 UI 验收](../../../scripts/acceptance/project/comments.js) |

相关 store、control、cli crate 的 `cargo clippy --all-targets -- -D warnings` 通过。仅执行受影响链路，未运行全量测试。发布包、真实浏览器及线上观察的结果由执行回执记录。

相关说明：[项目功能](../../project/index.md)、[项目逻辑](../../../agents/project/index.md)、[项目规则](../../../rules/07-project-module-contract.md)。
