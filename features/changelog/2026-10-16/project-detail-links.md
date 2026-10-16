Commit: f8cb8fdf60333fbbeb896355a0732501bef908e0

# 项目、子项与 TODO 详情分享链接

详情展开状态进入 URL，复制当前地址后，其他用户可登录并直接打开指定项目、子项或 TODO。刷新和新标签页保留定位；嵌套详情保留父级，浏览器前进后退切换详情。关闭清除对应地址参数，显式切换菜单离开详情。

仅目标 ID 即可定位，独立子项、未归属 TODO 同样支持；片段优先，兼容同名查询参数，保留无关地址参数。加载失败保留目标并允许重试，记录不存在另行提示。未保存任务、评论和编辑草稿接入页面离开确认；返回确认前固定地址快照，防止浏览器恢复历史条目期间的界面更新提前卸载表单。

影响范围为 SPA 导航、项目详情和共用草稿保护；服务端接口、数据结构与权限沿用现有规则。

定向验证覆盖 12 个测试文件中的 61 个用例，全部通过；仅选择相关用例，未运行全量测试。浏览器使用实际 SPA 产物通过 7 组导航与草稿检查，四种屏宽无外层横向溢出，未发生接口写入或页面异常。`typecheck:ontology`、SPA 构建及独立构建产物一致性检查通过。

## 功能与测试

| 功能 | 验证入口 |
| --- | --- |
| 编解码、嵌套参数、无关参数保留 | `project/navigation/links.test.js` |
| 三层直接打开、历史切换、关闭、失败重试 | `project/navigation/navigation.dom.test.jsx` |
| 分享地址优先于持久化主页、菜单清除定位 | `appProjectLinks.dom.test.jsx` |
| 历史事件到达前及恢复间隙保留草稿、保存中禁止返回、跨页历史顺序 | `fleet/navigation/history.dom.test.jsx` |
| 登录保留目标、执行与 Brain 地址更新保留历史顺序 | `appProjectLinks.dom.test.jsx`、`app.dom.test.jsx` 的 URL 登录用例、`brain/workbench/tests/navigation.dom.test.jsx` |
| 既有执行链接、菜单、编辑与详情交互 | `fleet/navigation/links.dom.test.jsx`、`appNavigation.dom.test.jsx`、`ontology/navigation/DraftGuard.test.tsx`、`project/project.dom.test.jsx`、`project/tests/todoTabs.dom.test.jsx`、`project/forms/editor.dom.test.jsx` |
| 实际 SPA 的登录、刷新、新标签页、前进后退、草稿与错误分支 | [navigation.js](../../../scripts/acceptance/project/navigation.js)；使用隔离只读 HTTP 数据，检查 1920/1280/768/390 四种屏宽 |

相关说明：[项目工作台](../../project/index.md)、[Web 导航](../../../agents/web/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。
