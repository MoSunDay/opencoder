Commit: 5a463f3b596a99bf04a09e0464a2a3d94af97dbf

# Team 执行记录与进行中的讨论

原页面只展示已完成轮次，启动后即使成员已回答，用户仍看不到过程。Team 页面新增“Team 列表／执行记录”切换；启动提交中、受理成功和失败分别反馈，受理后自动打开执行详情。历史记录按服务端游标分页并自动刷新，点击记录打开原执行，不重新派发。提交结果不确定时保留输入及同一执行 ID 重试。

过程组件读取当前轮次已保存的计划、成员发言和队长小结，自动跟进澄清及下一轮；尚未生成的记录显示等待，读取失败可重试。失败、取消或中断后仍可查看已保存的未完成轮次。轮询不重叠、单次发现的小结数有上限，切换执行取消旧请求；大记录继续使用手动分段读取。

## 实现与验证

- [Team 子组件](../../../crates/web/spa/src/fleet/teams/) 分离列表、编辑、启动与 `TeamExecutionHistory`，后者可嵌入其他页面。
- [TeamExecutionProcess](../../../crates/web/spa/src/fleet/detail/team/index.jsx) 由通用执行详情复用；[progress.js](../../../crates/web/spa/src/fleet/detail/team/progress.js) 负责未完成轮次发现，复用现有有界记录 API。
- [页面测试](../../../crates/web/spa/src/fleet/teams/teams.dom.test.jsx) 覆盖历史重开、受理反馈、防重复提交、同 ID 重试、分页、错误与请求取消。
- [过程测试](../../../crates/web/spa/src/fleet/detail/team/conversation.dom.test.jsx) 覆盖整轮完成前展示发言、补充澄清、下一轮、失败保留输出、等待与错误区分、有界读取及切换取消。
- [UI 验收](../../../scripts/acceptance/ui/scenarios/teams.js) 在执行完成后从历史重新打开同一记录。

SPA 全量 155 个测试文件、1107 项测试及前端构建通过。隔离 Server 与 Agent 使用受控模型响应，实际完成启动、两轮讨论、一次补充澄清、最终总结和历史重开；四种屏宽通过。格式检查通过；Rust 工作区检查、测试和构建在编译阶段超时，发布脚本回归也超时，未计为通过。本次只调整前端及验收代码。
