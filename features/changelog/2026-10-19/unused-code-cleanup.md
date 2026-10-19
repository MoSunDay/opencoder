Commit: 9a8213e3dba80353fefcf7d20aa57bf7461f27c8

# 删除未使用实现与依赖

## 变化

- 删除未被现有界面调用的 `teamModals.jsx`、`teamItems.js`、`promptEditor.jsx`、`project/execute/result.jsx`、`project/views/relationSelect.jsx` 和 `ontology/ui/useInitialLoad.tsx`，以及仅测试旧实现的用例。
- 删除导航、资源映射、旧事件展示等无调用辅助代码；归属编辑和里程碑删除测试改为覆盖当前界面及编辑入口。相对本次开始时的工作树，SPA 生产源码净减少 694 行。
- 删除 12 项 Rust 和 2 项前端直接依赖声明，保留依赖版本及 Rust 最终启用的 feature。项目模块的 3 项测试依赖和 CLI 的 `async-trait` 被跨目录测试使用，继续保留。
- 清除仓库说明、历史变更记录及发布说明对特定记忆维护技能的引用，更新当前界面入口与测试索引，简化重复的实现细节。
- 产品功能、历史数据兼容和运行时上下文保持原样；本次没有修改 Rust 源码、数据库、模型请求或内置技能。

## 功能与测试

| 保留的行为 | 定向测试 |
| --- | --- |
| Agent 资源草稿保留、读取失败禁止覆盖 | [agentDetail.dom.test.jsx](../../../crates/web/spa/src/agentDetail.dom.test.jsx) |
| Team 编辑与执行入口 | [teams.dom.test.jsx](../../../crates/web/spa/src/fleet/teams/teams.dom.test.jsx) |
| TODO 归属搜索、清空及切换记录时隔离旧请求 | [editor.dom.test.jsx](../../../crates/web/spa/src/project/forms/editor.dom.test.jsx) 的两个新增用例 |
| 原生执行过程、离线错误及实时结果读取 | [liveTranscript.dom.test.jsx](../../../crates/web/spa/src/fleet/detail/liveTranscript.dom.test.jsx)、[dispatch.rs](../../../crates/control/tests/e2e/project_links/dispatch.rs) 的 `results_are_read_from_the_owner_and_report_node_failures_without_cached_success` |
| 里程碑与节点删除、导航选项和页面标题 | [model.test.js](../../../crates/web/spa/src/brain/workbench/milestone/model.test.js)、[nav.test.js](../../../crates/web/spa/src/nav.test.js)、[headerContract.dom.test.jsx](../../../crates/web/spa/src/shell/headerContract.dom.test.jsx) |

## 验证

- SPA 定向测试：23 个文件、218 项通过；结果接口定向测试：1 项通过。
- Ontology TypeScript 检查通过；SPA 构建通过，`scripts/check-spa-drift.sh` 确认产物一致。
- 对 agent、cli、dag、dag-runtime、llm、node、project、server、session、store 执行离线、锁定依赖的 Clippy `--all-targets -- -D warnings`，通过且无警告。
- 493 个前端源码及测试文件的相对导入有效；修改文档的本地链接与差异格式检查通过。未运行全量测试，未发布。
