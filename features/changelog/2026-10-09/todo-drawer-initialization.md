# 新建 TODO 详情初始化

新建 TODO 后，概览记录到达前显示读取状态；记录到达后初始化标题、说明、专项和看板列。后续概览刷新保留未保存的编辑，避免空表单阻止保存。

| 功能 | 测试 |
| --- | --- |
| 等待新记录、保留后续编辑并保存原归属与状态 | `waits for a newly created TODO before seeding its edit buffer and preserves later edits`，`crates/web/spa/src/project/views/editing.dom.test.jsx` |
| 新建 TODO、选择能力并保存与派发 | `scripts/acceptance/project/main.js` |
