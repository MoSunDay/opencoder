Commit: 00f0d5d3036841c6bac43069c4e08d8c8288fe38

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

## 验证与发布

- 6 个相关测试文件共 28 例通过，范围为父级解析、地址导航、页面入口、浏览器历史与项目详情；未运行全量测试。
- 构建后的 SPA 浏览器验收 9 组通过：登录定位、刷新、新标签页、独立/未归属详情、逐层关闭、连续 Escape、前进后退、未保存草稿及错误重试。
- 79.43 公共入口使用真实项目、子项和 TODO 完成 6 组浏览器验收。1920、1280、768、390 四种宽度通过；关闭 TODO 显示子项，关闭子项显示项目，返回项目保留页签。页面脚本错误与业务写入均为 0。
- 成套 7 个二进制构建及摘要校验通过，SPA 独立重建无差异。信号发布版本为 `00f0d5d3036841c6bac43069c4e08d8c8288fe38`。
- 发布后观察 900 秒，188 次公共入口、就绪状态、总览及 TODO 读取检查无失败，最大采样间隔 5.01 秒；资源服务进程未改变。旧 Runtime 最终剩余任务为 0，进入正常退役，无退役错误。
- 证据目录：`/data00/workspace/artifacts/opencoder-project-hierarchy-20261016`，最终回执为 `delivery-receipt.json`，线上截图及断言为 `browser-production/result.json`。
