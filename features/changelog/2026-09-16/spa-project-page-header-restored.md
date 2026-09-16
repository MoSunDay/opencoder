Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# SPA 项目页页头恢复：project 回到 PAGE_META 并真挂 PageShell，menu-only 契约补齐侧栏一半 + 注释去硬编码计数

## 背景

66295a68 把 project 页登记进 `HEADERLESS_REASONS`（理由 `body-title`：页名由面板 antd Tabs 承担）并删除了 `project/project.jsx` 里的 PageShell 死导入，舰队控制台「项目」页从此没有页头（H1 等价物）。用户要求找回该页头，复用既有 PageShell + `nav.js` `PAGE_META` 机制，文案逐字恢复 66295a68 之前的历史版本（`title: '项目'`、`desc: '目标、里程碑与 TODO 的用户策展跟踪'`）。fe00626e 对 menu-only 页「不加冗余页头」的决策不受影响——project 不是 menu-only 页。评审同时提出 4 处测试/注释加固。

## 决策与改动

- `spa/src/nav.js`：`HEADERLESS_REASONS` 删除 `project: 'body-title'`（注册表余 8 页）；`PAGE_META` 以 `project` 为第一键新增条目（历史文案逐字）；`PAGE_META` 文档注释同步为 project / progress / ownerview。`NAV_CATEGORIES` 等其余字节不变。
- `spa/src/project/project.jsx`：`ProjectPanel` 用 `<PageShell page="project">` 包住原外层 `<div>`（error Alert + Spin/Tabs + TodoDrawer，内层不动），导入真实接线（不再是 1f66430f 式死导入）；门面注释补一句「页头（title+desc）来自 nav.js PAGE_META」。
- `spa/src/shell/pageShell.jsx`（纯注释）：门面注释不再硬编码「8 挂载点 / 2 渲染页头 / 另外 6 个」这类会静默腐烂的计数，改为「渲染页头的挂载点正好是 PAGE_META 全集（project / progress / ownerview）；其余挂载点（权威名单 `grep -rn '<PageShell' src`）是无标题包装」，6 个包装的枚举保留。
- `spa/src/shell/pageShell.dom.test.jsx`（纯注释）：extra-only 用例内 “6 of the 8” → “most PageShell mount points”。
- `spa/src/shell/headerContract.dom.test.jsx`（纯注释）：顶部第 1 条 “only progress / ownerview” → “only project / progress / ownerview”。逐页断言本就注册表驱动，自动适配：project 页现在走 PAGE_META 分支，`.oc-page-title` / `.oc-page-desc` 与文案逐字相等由该套件继续咬合。
- `spa/src/app.dom.test.jsx`：`names every menu-only page in the mobile page Select` 三处加固——① `menuOnly.length` sanity 断言带失败信息（将来若没有 menu-only 页，应删用例而不是放宽）；② 补齐 menu-only 契约的另一半：除移动端 Select 标签外，断言侧栏 Menu 真的渲染同名菜单项（`.fleet-sidebar` 范围内、图标字形自带 aria-label → 按转义后的正则匹配）；③ 加耦合说明：该用例挂载真实 team/topics/chat/nodes 面板，面板挂载失败或 antd deprecation（文件级 afterEach 上报）不是导航契约失败，先读面板自身套件。顶部 nav 导入注释同步为「两个 DOM 半侧都在此断言」。
- `spa/src/project/project.dom.test.jsx`：新增 `renders the PAGE_META page header above the four tabs` 作为 `describe('ProjectPanel')` 第一个用例：`.oc-page-title` = '项目'、`.oc-page-desc` = '目标、里程碑与 TODO 的用户策展跟踪'（沿 `progressPanel.dom.test.jsx` 的硬编码文案+注释指向 PAGE_META 风格，逐字钉住恢复的页头）、`getByRole('heading', { name: '项目' })`，并断言四个 tab 仍在页头之下渲染。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| project 页头逐字渲染（title+desc+heading）且四 tab 不受影响 | `renders the PAGE_META page header above the four tabs`（新增） | `src/project/project.dom.test.jsx` |
| project 面板真渲染 PAGE_META 文案（死配置回归） | `every page keeps exactly one title source`（注册表驱动，project 自动改走 PAGE_META 分支） | `src/shell/headerContract.dom.test.jsx` |
| PAGE_META ∪ HEADERLESS == ALL_PAGES、headerless 页不入 PAGE_META | `covers every page key exactly` 等（未改，自动适配） | `src/nav.test.js` |
| menu-only 双 DOM 面：侧栏 Menu 项 + 移动端 Select 标签 | `names every menu-only page in the mobile page Select`（加固） | `src/app.dom.test.jsx` |
| PageShell 单元契约（标题/extra/bare/headerless） | 全文件（仅一处注释更新） | `src/shell/pageShell.dom.test.jsx` |

## 验证

- 定向：`npx vitest run src/nav.test.js src/project/project.dom.test.jsx src/shell/ src/project/progressPanel.dom.test.jsx src/project/views/ src/app.dom.test.jsx` → **8 files / 94 tests 全绿，0 failed**（net +1 用例 = 新页头用例；headerContract 的 project 行改走 PAGE_META 分支后仍绿）。全量回归与 `spa/dist` 重建由发布关口另行执行（本轮不触碰共享树的在制品与 dist）。
- 插曲（非代码问题）：首轮定向运行期间共享树中 `nav.js` 被并发在制品回滚为 HEAD 版本，导致新用例一度失败（`.oc-page-title` 不存在）；重新应用本轮 nav.js 改动后连续两轮全绿。

稳定文档已同步：[Web 模块](../../../agents/web/index.md)（渲染页头挂载点 = PAGE_META 全集 project/progress/ownerview、HEADERLESS 注册表 8 页、menu-only 双 DOM 面守卫落点 `app.dom.test.jsx`）。[spa-page-shell-mount-truth.md](spa-page-shell-mount-truth.md) 记录的 8/2/6 计数为其当时事实，由本条目接续。
