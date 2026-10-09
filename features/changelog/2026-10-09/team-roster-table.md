Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# Team 成员选择结果表格

Team 编辑弹窗在选定队长和成员后，以“名称 / 角色 / 描述”表格展示实时名单。队长置顶并标记角色，超长名称和描述单行省略并保留完整文本提示；窄屏下表格在容器内横向滚动，弹窗保持屏幕内宽度。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 选中成员后的三列表格、队长角色、长描述省略提示与滚动容器 | `builds a captain-first roster from agent identity and posts deduped members` | `crates/web/spa/src/fleet/fleet.dom.test.jsx` |
| 队长置顶、成员去重和描述映射保持纯函数 | `keeps roster mapping pure and captain-first when selections contain duplicates` | `crates/web/spa/src/fleet/fleet.dom.test.jsx` |
