Commit: 8c897b3086d540f5f66d5388f15c12b924bdcefd

# 能力库长类型标签不再覆盖描述列

能力库允许显示已有的自定义执行类型。固定列宽下，未限宽的长标签会越过单元格，覆盖相邻描述；只检查描述省略和页面总宽度无法发现这个问题。

执行类型标签现在限制在本列内容宽度内，长名称显示省略号，悬停保留完整文本。手机上的操作列随表格横向滚动，避免固定列占据描述区域。列表和搜索结果使用相同布局，保存及执行目标绑定不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 长英文、自定义中文类型的边界、完整悬停文本和描述省略 | `measure` / `checkWidth` | [layout.js](../../../scripts/acceptance/brain/ui/layout.js) |
| 四种屏宽、横向滚动、搜索结果和完整详情表单 | `--only-capabilities` | [layout.js](../../../scripts/acceptance/brain/ui/layout.js) |
| 列表、搜索、新建、编辑、删除确认和错误重试 | `capability library table and editor` | [brainPanel.dom.test.jsx](../../../crates/web/spa/src/brainPanel.dom.test.jsx) |
| 工作台能力标签页和新建、编辑入口 | `brain workbench capability tab` | [capabilities.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx) |

- 旧构建在新增浏览器断言下失败：执行类型覆盖下一列。
- 定向前端测试：2 个文件、13 项通过。
- 浏览器布局验收：1920、1280、768、390 四种屏宽，16 项通过；使用隔离的确定性接口数据，不等同于调度执行验收。
- `npm run build` 通过。
- 测试范围仅能力库及其直接交互；未运行全量测试，未修改 Rust。

参见 [Brain 功能](../../brain/index.md) 与 [Web 模块](../../../agents/web/index.md)。
