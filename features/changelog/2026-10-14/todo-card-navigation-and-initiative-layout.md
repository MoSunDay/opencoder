Commit: 8b25bde61ba3c8a44ee7bad187563084675d86d8

# TODO 标题状态、执行跳转与专项阅读布局

任务状态放在任务标题输入框右侧，同一行修改，随底部“保存 TODO”提交。任务页签文字不再附加状态，设置页不重复提供状态字段。新建 TODO 使用同样的标题和状态布局。

执行记录移除“查看”按钮，执行 ID 直接在新页面打开原生详情，按实际执行类型展示；执行链接优先于上次保存的菜单位置，刷新后仍能定位历史记录。原任务及评论草稿不会因打开链接而保存或丢失。

看板卡片按标题、Markdown 可读摘要、标签和负责人排布，删除收在更多操作菜单内，保留拖拽手柄及确认删除。长内容在卡片内换行，空列有明确提示。键盘拖拽期间 Esc 只取消拖拽，不连带关闭专项；取消后恢复抽屉原有 Esc 行为。

专项先展示当前概述和专项说明，再靠左展示所属项目、人员与进度。编辑和新建专项明确提供靠前的“专项说明（Markdown）”，仍写入原 detail_md 字段；只修改说明不提交未改动的概述。

## 测试覆盖

| 功能 | 测试名或入口 | 文件 |
| --- | --- | --- |
| 标题、状态同行保存，设置不重复状态 | `edits status beside the task title and persists both through the footer save` | [todoTabs.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoTabs.dom.test.jsx) |
| 原生执行 ID 链接及未知类型 | `displays execution type, name and ID resolved from the index`、`preserves a missing execution ID but does not open an unknown detail type` | [todosTab.dom.test.jsx](../../../crates/web/spa/src/project/todosTab.dom.test.jsx) |
| 六类历史执行定位及关闭后清理 URL | `opens a linked %s record even when absent from the first list page` | [links.dom.test.jsx](../../../crates/web/spa/src/fleet/navigation/links.dom.test.jsx) |
| 执行链接优先于菜单保存值 | `opens an execution link before restoring a saved project page` | [appNavigation.dom.test.jsx](../../../crates/web/spa/src/appNavigation.dom.test.jsx) |
| 卡片阅读、删除确认、只读、Markdown 摘要 | 5 个卡片定向用例 | [card.dom.test.jsx](../../../crates/web/spa/src/project/board/card.dom.test.jsx) |
| 专项说明编辑与布局顺序 | `initiative detail retains TODO board states and edits without initiative state` | [initiativeStatus.dom.test.jsx](../../../crates/web/spa/src/project/views/initiativeStatus.dom.test.jsx) |
| 四种屏宽、真实 CRUD、重试与评论 | `forms.js` 的 TODO 表单检查 | [forms.js](../../../scripts/acceptance/project/forms.js) |
| 新页面原生组件、刷新及草稿保留 | `verifyExecutionLinks` | [design/links.js](../../../scripts/acceptance/project/design/links.js) |
| 真实拖拽保存、键盘取消和专项说明保存 | `verifyDetails`、`verifyCards` | [interactions.js](../../../scripts/acceptance/project/interactions.js)、[design/cards.js](../../../scripts/acceptance/project/design/cards.js) |

- 定向 SPA 检查：51 个不同用例通过；修改 Esc 处理后仅重跑直接影响的 7 项。未修改 Rust，未运行 Rust 测试或 Clippy。
- 浏览器覆盖 1920、1280、768、390 像素。执行导航仅模拟执行读取，项目和专项的创建、保存、拖拽、评论与 CLI 使用独立 Server 的实际接口。
- 首轮浏览器检查发现 Esc 连带关闭专项，保留失败记录；修复后只回归专项和卡片，以及补充执行新页面异常检查。未运行全站或全量测试。
- 本机交付使用正式成套发布包、正常信号切换和 900 秒只读观察；实际版本及结果保存在仓库外发布回执，不将此前原生长任务验收的 504 记录改写为通过。

相关说明：[项目工作台](../../project/index.md)、[Web 模块](../../../agents/web/index.md)、[项目规则](../../../rules/07-project-module-contract.md)。
