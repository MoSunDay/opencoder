Commit: a4e0d53645cede306c71663f839ba6e989c21cc3

# 项目现状与目标折叠及子项名称

项目、子项详情中的现状和目标分别提供展开、收起按钮，默认收起正文。现状时间与修改记录入口保持可见；展开保留完整 Markdown，刷新同一记录保留展开状态，切换记录重新收起。

网页统一使用“子项、目标、现状”，覆盖列表、详情、创建与编辑、所属关系、Tag 和版本入口。`initiative`、`detail_md`、`current_summary`、`summary_updated_at` 及 CLI 命令沿用现有实现。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 项目、子项分别折叠现状与目标，刷新保持展开 | `%s and summary collapse independently and retain expansion after refresh` | [initiativeStatus.dom.test.jsx](../../../crates/web/spa/src/project/views/initiativeStatus.dom.test.jsx) |
| 切换记录重置折叠 | `resets summary and description folds when switching initiatives` | [initiativeStatus.dom.test.jsx](../../../crates/web/spa/src/project/views/initiativeStatus.dom.test.jsx) |
| 现状显示、时间、清空、筛选与编辑 | `shows the saved summary and its own timestamp with an explicit empty state` 等 | [summary.dom.test.jsx](../../../crates/web/spa/src/project/summary/summary.dom.test.jsx) |
| 历史记录与恢复入口 | `reads history on demand and restores a selected version after confirmation` 等 | [history.dom.test.jsx](../../../crates/web/spa/src/project/summary/history.dom.test.jsx) |
| 子项列表、关系、Tag、表单及导航名称 | 对应模块定向用例 | `project` 与 `shell` 下本次调整的测试文件 |

- 定向回归：12 个相关测试文件，92 个用例通过，0 失败。
- SPA 生产构建通过；未修改 Rust，未运行全量回归。
- 本机发布后：真实记录和长文本显示样例分别验证项目、子项在 1920 / 1280 / 768 / 390 宽度的折叠、完整正文、键盘、历史与编辑入口，共 16 组通过。未写入现有项目数据。
- 79.43 同步同一发布包后，真实记录的项目、子项在四种宽度共 8 组验证通过，浏览器异常为 0，检查未写入项目数据；入口鉴权保持关闭。
- 发布版本 `a4e0d53645cede306c71663f839ba6e989c21cc3`：先本机验证，再同步到 79.43。两台分别完成 900 秒入口健康观察，失败请求为 0；切换前的执行归属、旧 Runtime 进程与资源服务均通过核验。
- 本轮证据：`/tmp/opencoder-collapse-20261015`；两台发布回执与在线备份：`/data00/opencoder-delivery/project-content-20261015`。未运行 Rust、全 SPA 或全站回归。
