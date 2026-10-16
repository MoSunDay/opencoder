Commit: 34bf9abe968e3dba88dbb79d5dbe08d6a5707392

# 分享详情关闭后返回所属父级

仅含 TODO 或子项 ID 的地址此前只展开目标详情，关闭后直接回到总表。现在按总览中的当前归属恢复父级：TODO → 子项 → 项目 → 列表。独立子项和未归属 TODO 只显示实际存在的层级；查询参数、刷新、新标签页与浏览器前进后退沿用同一规则。

父级恢复属于页面状态，不修改记录归属，不额外增加历史条目；过时的父级地址参数不能覆盖当前关系。加载中、读取失败和记录缺失保留原目标。同一时刻只打开最深层抽屉，关闭按钮与 Escape 均只返回一层，父级组件保留当前页签；未保存内容继续使用既有确认流程。

## 功能与测试

| 功能 | 测试 |
| --- | --- |
| 实际归属、过时父级、独立/未归属与异步加载 | `project/navigation/hierarchy.test.js` |
| ID-only 地址逐层关闭、查询参数和加载后恢复父级 | `project/navigation/navigation.dom.test.jsx` |
| URL 编解码、前进后退、草稿与页面入口 | `project/navigation/links.test.js`、`fleet/navigation/history.dom.test.jsx`、`appProjectLinks.dom.test.jsx`、`project/project.dom.test.jsx` |
| 真浏览器登录、刷新、新标签页、逐层关闭、Escape 和草稿保护 | [navigation.js](../../../scripts/acceptance/project/navigation.js) |

相关说明：[项目工作台](../../project/index.md)、[Web 导航](../../../agents/web/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。
