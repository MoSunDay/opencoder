Commit: 6ab6ec63595b45b7440f047d768fff7108a6ab04

# 项目工作台：专项与执行关联

项目页收敛为项目、里程碑、专项、TODO 四个视图。专项与里程碑同级，均可以不关联项目；TODO 可以归属任一分组，也可以不分组。TODO 编辑与执行记录改为右侧抽屉，执行记录仅保存执行 ID，类型、名称与状态从执行索引读取；可关联已有执行，也可从 Agent、Team、DAG、TODO 工作流和大脑原生界面发起并自动关联。项目页不再提供独立的计划、执行和回放操作。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 执行 ID 关联、去重与删除清理 | `todo_execution_links_store_only_ids_and_cascade_on_delete` | `crates/store/tests/project_relations.rs` |
| 可关联能力边界 | `only_agent_execution_capabilities_can_be_linked` | `crates/control/src/api/project_links.rs` |
| HTTP 关联往返与错误响应 | `todo_execution_links_roundtrip_and_validate_kind` | `crates/control/tests/e2e/project_links.rs` |
| 里程碑、专项独立分组与 TODO 归属 | `milestones_and_initiatives_remain_distinct_and_both_hold_todos`、`initiatives_and_milestones_are_siblings_and_hold_separate_todos` | `crates/store/tests/project_relations.rs`、`crates/control/tests/e2e/project_links.rs` |
| 独立专项与 TODO 导航 | `creates a standalone initiative without any project and navigates its TODO list` | `crates/web/spa/src/project/views/relations.dom.test.jsx` |
| 执行索引投影与 ID 关联 | `displays execution type, name and ID resolved from the index`、`links an existing execution ID without storing a duplicate snapshot` | `crates/web/spa/src/project/todosTab.dom.test.jsx` |

定向回归：SPA 项目与应用测试 7 文件、45 项通过；Store 迁移、分组、SQL DDL 与关联测试通过；Control 两项 HTTP 往返通过；SPA 生产构建通过。全量项目验收脚本和工作区测试未在本条记录中执行。
