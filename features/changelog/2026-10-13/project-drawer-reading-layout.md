Commit: c3ccc137d99920049981ef4be34fd6d1c348bb61

# 项目抽屉与 TODO 正文交互

用户要求项目抽屉桌面宽度改为 70%，专项移除页签，并取消专项和 TODO 的居中限宽布局；TODO 删除“编辑任务”，说明默认预览，切换编辑后通过底部保存。

- 项目桌面 70vw、窄屏全宽，保留概览与专项页签。
- 专项按人员与进度、概述与说明、TODO 看板连续展示；宽屏说明并排，窄屏纵向排列。
- TODO 的任务、设置、执行、评论页签保留。标题直接修改，任务说明默认预览 Markdown，用紧凑的预览/编辑控件切换，底部按钮统一保存。
- 表单仍使用单个 Ant Design Form。切换正文显示、页签和后台刷新保留草稿，保存失败允许重试；新建、发表评论的正文仍默认编辑。
- 调整抽屉间距、正文行距和手机底部按钮，内容铺满可用区域，表格和看板保留内部滚动。

## 验收映射

| 行为 | 测试入口 |
| --- | --- |
| 默认预览、无整页编辑按钮、切换保留草稿、底部保存、标题校验 | [todoTabs.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoTabs.dom.test.jsx)、[editor.dom.test.jsx](../../../crates/web/spa/src/project/forms/editor.dom.test.jsx)、[editing.dom.test.jsx](../../../crates/web/spa/src/project/views/editing.dom.test.jsx) |
| 无专项页签、看板直接可用、归属与新建不变 | [initiativeStatus.dom.test.jsx](../../../crates/web/spa/src/project/views/initiativeStatus.dom.test.jsx)、[relations.dom.test.jsx](../../../crates/web/spa/src/project/views/relations.dom.test.jsx)、[project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 执行页按需读取与异步响应隔离 | [todoDrawer.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoDrawer.dom.test.jsx)、[todosTab.dom.test.jsx](../../../crates/web/spa/src/project/todosTab.dom.test.jsx) |
| 真实创建编辑、失败重试、四种屏宽与项目 70% | [forms.js](../../../scripts/acceptance/project/forms.js)、[interactions.js](../../../scripts/acceptance/project/interactions.js) |
| 连续专项布局中的历史概述、筛选拖动与看板草稿 | [project_workbench_ui.js](../../../scripts/acceptance/project_workbench_ui.js) |

相关：[项目工作台](../../project/index.md)、[Web 实现](../../../agents/web/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。

验证结果：8 个相关 SPA 测试文件共 35 项通过；真实 Server/CLI 创建、编辑、失败重试、评论和四种屏宽共 40 组布局检查通过，浏览器异常为 0。专项历史与看板筛选拖动验收通过。首轮窄屏宽度断言失败，修正 Drawer 外层样式后通过；未运行全量测试。
