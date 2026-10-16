Commit: 850192cd62bb595d6dee52ffbafb0c41a66d49b9

# TODO 卡片直接拖动

移除 TODO 卡片的独立拖动按钮，标题、说明与卡片空白处都可以直接拖动，鼠标沿用六像素启动距离，手机长按 200ms 后拖动，普通滑动仍可滚动；沿用已有状态、顺序保存。点击标题与 Enter 打开详情，空格开始键盘排序，Escape 取消且不关闭专项；更多菜单不参与拖拽，只读用户仍可阅读任务。

| 行为 | 定向验证 |
| --- | --- |
| 标题打开、说明与只读显示、菜单确认删除 | `card.dom.test.jsx` |
| 专项内搜索后仍能打开和拖动任务 | `project.dom.test.jsx` 的 `opens project progress` |
| 标题和说明拖动、保存失败回退、菜单独立、键盘取消和 Enter 打开 | `scripts/acceptance/project/forms.js ... cards`，独立 Server |
| 筛选隐藏顺序、多标签重复卡片同步、失败回退 | `project_workbench_ui.js --drag-only`，浏览器受控 API |
| 四种宽度无外层溢出、标题与菜单不重叠 | 同一 `cards` 验收的 1920、1280、768、390 截图与测量 |

发布沿用当前本机版本，执行配套构建、信号切换和 900 秒只读观察。实际结果与证据写入发布回执，不将定向验收表述为全量通过。

相关：[项目工作台](../../project/index.md)、[web 模块](../../../agents/web/index.md)。
