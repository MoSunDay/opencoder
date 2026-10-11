Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# TODO 表单布局

TODO 编辑表单的输入项原先放在同一个布局子节点中，字段之间缺少独立间距。新建、编辑与执行参数表单现在共用分区、字段网格和操作区，按任务内容、归属与状态、人员和执行设置组织；每个控件都有可见标签。内容区在宽屏居中，手机切为单列，说明自动增高，长标签与关注人换行。

新建和保存按钮固定在抽屉底部，请求失败提示与操作按钮一起展示，保留草稿；提交中禁用字段。空标题显示字段错误并定位到输入框。关联执行输入框与指派表格留出独立间距，窄屏表格在内部横向滚动。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| 分区、标签关联与底部操作 | `gives every editor control a visible associated label inside a named form section` | [表单交互测试](../../../crates/web/spa/src/project/forms/editor.dom.test.jsx) |
| 空标题、提交禁用、刷新保留和失败重试 | `retains failed edits and prevents field changes while a save is pending` | [表单交互测试](../../../crates/web/spa/src/project/forms/editor.dom.test.jsx) |
| 新建失败保留、错误位置、只读禁用 | `keeps create errors and drafts in place...`、`keeps read-only fields...` | [表单交互测试](../../../crates/web/spa/src/project/forms/editor.dom.test.jsx) |
| 归属、状态、Tag 编辑与刷新 | `editing.dom.test.jsx` | [已有编辑测试](../../../crates/web/spa/src/project/views/editing.dom.test.jsx) |
| 人员字段与执行关联、派发重试 | `fields.dom.test.jsx`、`todoDrawer.dom.test.jsx`、`todosTab.dom.test.jsx`、`launcher.dom.test.jsx` | [人员](../../../crates/web/spa/src/project/people/fields.dom.test.jsx)、[关联](../../../crates/web/spa/src/project/tests/todoDrawer.dom.test.jsx)、[列表](../../../crates/web/spa/src/project/todosTab.dom.test.jsx)、[派发](../../../crates/web/spa/src/project/execute/launcher.dom.test.jsx) |
| 四种屏宽、长内容、多选、真实新建编辑、失败重试、刷新持久化 | `project/forms.js` | [独立 Server 浏览器验收](../../../scripts/acceptance/project/forms.js) |

## 定向验证

- 相关 SPA 测试共 6 个文件、34 个用例通过；最后修改后重跑受影响的 3 个文件、12 个用例通过。未运行全量回归。
- `npm run build` 与 `cargo build -p opencoder-server -p opencoder-cli` 通过，Server、ctl 与最终 SPA 摘要一致；构建使用源码快照，回填产物前核对当前 SPA 源码未漂移。
- 独立真实 Server 验收通过：新建、编辑、执行参数三类表单在四种屏宽下无重叠和外层溢出，字段间距、标签关联、多选完整展示和底部操作位置均符合预期；新建与编辑失败保留草稿，重试写入、刷新持久化及专项内新建通过，浏览器异常为 0。能力目录显示与错误响应使用夹具，未派发执行。
- `git diff --check` 通过。仅更新代码和本地产物，未部署生产。
- 功能与入口索引：[项目工作台](../../project/index.md)、[Web](../../../agents/web/index.md)、[验收说明](../../../scripts/acceptance/project/README.md)。
