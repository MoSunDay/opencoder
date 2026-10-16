Commit: a9fd4849ac23feaa692f22b2a2ef4ee8b88c68ca

# TODO 任务内评论与紧凑执行栏

用户决定移除评论页签，评论直接放到任务说明底部；执行能力选择与提交同行，移除已有执行 ID 输入；状态紧挨任务页签标题同行展示。

- TODO 保留任务、设置、执行三个页签。任务页展示说明及评论，切换页签保留任务和评论草稿。
- 保存 TODO 与发表评论各自提交，互不提交或覆盖另一份草稿。评论源执行仍可跳到执行页，隐藏任务页停止评论读取。
- 执行能力选择与指派按钮在桌面和手机均同行，指派记录、派发后自动关联和失败重试保留。已有执行仍可用 API/CLI 关联，网页移除手动输入。
- 状态放在“任务”页签文字右侧，评论输入框使用三行起始高度。

## 验收映射

| 行为 | 测试入口 |
| --- | --- |
| 内嵌评论、独立保存草稿、来源跳转、关闭确认 | [todoTabs.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoTabs.dom.test.jsx) |
| 派发后的异步读取、保留指派记录、取消旧读取 | [todoDrawer.dom.test.jsx](../../../crates/web/spa/src/project/tests/todoDrawer.dom.test.jsx)、[todosTab.dom.test.jsx](../../../crates/web/spa/src/project/todosTab.dom.test.jsx) |
| 单表单保存失败、只读权限、可见字段标签 | [editor.dom.test.jsx](../../../crates/web/spa/src/project/forms/editor.dom.test.jsx) |
| 评论重试幂等、分页、隐藏时停止读取 | [panel.dom.test.jsx](../../../crates/web/spa/src/project/comments/panel.dom.test.jsx) |
| 真实保存与评论、四种屏宽状态和执行同行布局 | [forms.js](../../../scripts/acceptance/project/forms.js) 的 todo 范围、[interactions.js](../../../scripts/acceptance/project/interactions.js) |

相关：[项目能力](../../project/index.md)、[Web 模块](../../../agents/web/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。

验证结果：26 项相关 SPA 测试通过；真实 Server/CLI 保存、评论和四种屏宽共 32 组布局检查通过，浏览器异常为 0。真实数据只读预览 8 组检查通过。截图发现手机执行栏被表格撑宽，修正外层网格并加强按钮边界断言后通过。未运行全量测试。
