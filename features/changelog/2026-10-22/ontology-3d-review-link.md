Commit: 8a4025fa76a0086638dfc3243bbe4617f08cfc1a

# Ontology 三维观测链接与本地交付

三维图谱增加可直接打开指定切面和观测中心的地址。链接条件优先于浏览器记忆；中心与切面不匹配或对象已停用时提示并恢复可用范围。地址变化复用导航与草稿保护，离开图谱时清除观测参数，不写回保存的切面。

发布源码以本地当前版本为基线，只合入三维图谱及观测链接。保持既有数据库、执行和发布协议，完整保留已发布功能。

## 测试覆盖

| 行为 | 用例 | 文件 |
| --- | --- | --- |
| 地址解析、导航入口、参数清理 | `observation links` | `ontology/navigation/observationLink.test.ts` |
| 链接覆盖旧记忆、失效恢复、中心约束 | `opens a linked aspect and center ahead of previous browser observation` 等 | `graph/session/linkedObservation.test.tsx` |
| 原导航和草稿历史 | 既有地址与历史用例 | `fleet/navigation/links.dom.test.jsx`、`history.dom.test.jsx` |
| 真实页面自动打开切面与中心 | `observation_link` | `scripts/acceptance/ontology/graph.mjs` |

- `npm run typecheck:ontology` 通过。
- 链接、观测恢复和导航历史四个测试文件：19 passed / 0 failed。
- 图谱页面、切面与观测记忆三个测试文件：20 passed / 0 failed。
- `npm run build` 通过，保留固定单文件构建的体积提示。
- 未改 Rust，不运行全量测试。

## 本地发布与验收

- 本地入口已切换到 `8a4025fa`，发布阶段完成；构建来自干净提交，保持原有数据格式与执行协议。
- 正式包的独立图谱验收通过。浏览器准备阶段采用 180 秒冷启动预算，并等待实际节点数稳定后断言；验收脚本修正记录于 `5803c91f`，没有改变发布包的运行代码。
- 公共入口的 HTML、JavaScript 和 CSS 与正式构建逐字节一致。真实浏览器验证了观测链接、详情、旋转、导航离开、刷新恢复与移动布局；37 个相关前端源码文件与主工作区一致。
- 旧 Runtime 的历史清单读取超时曾阻断发布。保留失败回执，沿原发布记录恢复回滚；在原生接口确认没有运行任务后完成休眠，再重试发布。没有修改认证数据或强停运行任务。
- 跨版本连续性验收 **未通过**：299 次成功提交后出现 1 次提交失败；就绪检查成功 4802 次、失败 14 次。日志记录索引报告基准变化后节点连接关闭。已受理请求的最大耗时约 4.98 秒，不代表整次切换满足无失败要求。
- 发布后的独立观察通过：实际观察约 901.8 秒，完成 121 个原生探针任务，最大完成耗时约 11.65 秒。旧 Runtime、NFS 和 TODO 资源服务保持原进程；抽查的三个新任务均归属新版 Runtime。此结果不能覆盖或替代上述跨版本失败记录。

验收标准见 [UI 与发布验收约定](../../../rules/05-ui-acceptance.md)。本轮证据保存在 `/data00/opencoder-delivery/ontology-3d-20261022-0736/`，包括正式包、逐库备份、浏览器截图、发布失败与恢复记录。
