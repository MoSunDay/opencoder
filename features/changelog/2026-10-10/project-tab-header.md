Commit: 3e870567ca1d67d0d27ecab4c8550f64a9c2aca9

# 项目页移除重复头部

移除项目 Overview 顶部的“项目”和“项目、专项与 TODO 工作台”，正文直接从项目、专项、TODO 三个表格页签开始。导航、表格和抽屉沿用原有功能；前端产物已重建。

实现通过 [导航配置](../../../crates/web/spa/src/nav.js) 将项目登记为 `body-title`，移除对应 `PAGE_META` 文案，避免保留空的标题行。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 项目正文不显示重复标题、说明或空头部 | `renders project content without the redundant title, description or header gap` | [pageShell.dom.test.jsx](../../../crates/web/spa/src/shell/pageShell.dom.test.jsx) |
| 实际项目面板保留三个表格页签 | `starts the project workspace with its table tabs and no repeated page header` | [headerContract.dom.test.jsx](../../../crates/web/spa/src/shell/headerContract.dom.test.jsx) |
| 隐藏头部时保留正文并隐藏操作行 | `bare skips an extra actions row the same page would otherwise render` | [pageShell.dom.test.jsx](../../../crates/web/spa/src/shell/pageShell.dom.test.jsx) |

- 既有前端验证：168 个测试文件、1177 个用例通过；构建、源码/产物一致性及四屏宽页面检查通过。
- 本次独立候选 `eaa30f17` 基于原线上版本，仅带头部调整：导航、PageShell、项目操作和真实面板头部共 57 个定向用例通过；四屏宽项目检查通过；7 个发布程序构建及摘要校验通过。此次仅修改前端，按最新仓库规则执行定向回归。
- 发布期间另一项维护发布已包含本改动，因此在暂存前停止独立候选发布，释放自身验证任务的等待标记，保留其日志；独立候选未部署。
- 改动已随整合版本 `3e870567` 上线。该版本 SPA 摘要 `4e2ab4c7c1ec46e46d4198f1b971e597699bbd8cbd75c16437cbf1073bd3fc07` 与前期通过检查的产物一致。
- 线上四屏宽（1920、1280、768、390）项目页验证通过：无重复头部、三个页签可切换、外层无溢出。用户权限页的一次性 Token 提示及创建窗口通过只读检查；未创建用户或改动已有 Token。
- 线上 15 分钟只读观察通过：入口、项目概览、公开版本和静态资源摘要共 175 次检查无异常，单轮最长 5.11 秒。最终回执见 `delivery.json` 与 `online-verification.json`。
- 实施与前期验证：`/var/tmp/opencoder-project-header-20261010/`；本次构建、整合发布对应关系、线上截图与观察：`/var/tmp/opencoder-project-header-publication-20261010/`。

相关索引：[Web 模块](../../../agents/web/index.md)、[项目工作台](../../../features/project/index.md)。
