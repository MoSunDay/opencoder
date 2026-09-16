Commit: 66295a68

# SPA 页头契约对齐真实渲染：PAGE_META 只留真渲染页，9 个无页头页逐页登记理由并被逐面板挂载看守（零 UI 变化）

## 背景

评审发现 `nav.js` 的 `HEADERLESS_PAGES = ['brain', 'topics', 'team', 'agents', 'nodes']` 注释与事实矛盾——「页面自带标题」只对 brain/agents 成立，topics/team/nodes 是 fe00626e 有意去掉的「冗余页头」，与页内标题无关；且没有任何测试咬住这个清单：`nav.test.js` 只做集合一致性检查（`PAGE_META ∪ HEADERLESS == ALL_PAGES`），任意页被塞进 `HEADERLESS_PAGES` 都会静默失去用户可见页名而全套测试依旧全绿。

进一步取证（在 jsdom 里挂载全部 11 个面板、逐页检查 `.oc-page-title`）发现更深一层的问题：`PAGE_META` 中 project / dag / todos / chat 四条标题+描述**从未被渲染过**，是死配置——只有 `progress`（project/progressPanel.jsx:160）与 `ownerview`（project/ownerViewPanel.jsx:145）两个面板真的**以自己的 page key 渲染出页头**（**勘误**：本条原文写作「只有这两个面板挂了 PageShell」，把「渲染页头的挂载点」与「挂载点」混为一谈——PageShell 实际有 8 个挂载点，另外 6 个是无标题包装，详见 `spa-page-shell-mount-truth.md`）；1f66430f 给 `dagPanel.jsx` 与 `project/project.jsx` 加了 PageShell 导入但从未接线。body 内自带 antd Tabs 标题的页：project / dag / todos / agents / brain；既无页头也无 Tabs 的全幅运营页：topics / team / nodes / chat（页名由侧栏 Menu 与移动端 Select 标签承担，fe00626e 已把这类页头判定为冗余并有测试锚定：team.dom.test.jsx:161,238、fleet/fleet.dom.test.jsx:29）。

## 决策

- **不回退** fe00626e 有意去掉的「冗余页头」（team.dom.test.jsx / fleet.dom.test.jsx 的断言仍锚定），也**不给** project/dag/todos/chat 四页新增页头——那会改变 UI，且与 fe00626e 的方向相反。
- 改为让 `nav.js` 如实登记，零 UI 变化：`HEADERLESS_REASONS` 按 IA 菜单顺序登记 9 页（`body-title`：project / brain / dag / todos / agents；`menu-only`：topics / team / chat / nodes），`HEADERLESS_PAGES = Object.keys(HEADERLESS_REASONS)` 派生，两表不可能漂移；`PAGE_META` 只保留真的以自身 key 挂 PageShell 的 progress / ownerview，死文案随删除一并消失。
- 删除 `dagPanel.jsx` 与 `project/project.jsx` 的两处死 `import { PageShell }`（删前 grep 确认各文件 `PageShell` 只出现在 import 行）。

## 契约测试如何咬住两类回归

`spa/src/shell/headerContract.dom.test.jsx` 从「按理由分组渲染 5 页」升级为「按页面逐个挂载全部 11 页」（describe `every page keeps exactly one title source`）：

- **死配置**：PAGE_META 页的面板必须真的渲染出 `.oc-page-title` / `.oc-page-desc` 且文案逐字相等——PAGE_META 里出现一条没有面板渲染的文案，该页会落入 headerless 分支（title 必须为 null）而失败；反向地，把某页塞进 HEADERLESS 并从 PAGE_META 删掉，若其面板其实没有 body 标题也会失败（反向探针 a 实证）。
- **静默失标题**：headerless 页必须无 `.oc-page-title`、理由必须已知、菜单标签必须非空；且两类理由**互斥**——`body-title` 页必须渲染 body 内标题（`.ant-tabs-nav` / `[role=tablist]` / h1-h6），`menu-only` 页必须**不**渲染任何 body 内标题。没有互斥断言时，把 project 的理由从 `body-title` 翻成 `menu-only` 会被无声接受（循环用例只要求无页头+标签在）；加了互斥后该方向同样失败（反向探针 b 实证）。
- **`shell/panels.jsx` 抽取的原因**：`main.jsx` 在 import 期自动挂载 `<App/>`，契约测试无法安全 import；`PANELS`（store `page` → 面板组件）抽成独立模块后测试可挂载任意单页，并以「键集 == IA 全页面集且都是组件」防漏页。
- api mock 按响应形状返回空列表（`{ nodes: [], teams: [], ..., goals: [], backlog: [], templates: [], workflows: [], ... }`）而非 `{}`：`fleet/teams.jsx` 把 `b.nodes`、brain 工作台把 `p.plans` 直接塞进 state，`{}` 会在面板内部 `undefined.map` 崩掉——本契约考的是标题来源，不是面板对畸形响应的容忍度，被测面板代码零改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| PAGE_META 页真的渲染标题+描述（progress / ownerview 挂真实面板） | `progress keeps exactly one title source`、`ownerview keeps exactly one title source` | `src/shell/headerContract.dom.test.jsx` |
| `body-title` 页无页头且必须渲染 body 内标题（project / brain / dag / todos / agents 挂真实面板） | `project keeps exactly one title source` 等 5 条 | `src/shell/headerContract.dom.test.jsx` |
| `menu-only` 页无页头且必须不渲染 body 内标题（互斥；topics / team / chat / nodes 挂真实面板） | `topics keeps exactly one title source` 等 4 条 | `src/shell/headerContract.dom.test.jsx` |
| 每页菜单标签非空（侧栏 Menu / 移动端 Select 即页名兜底） | 上述 11 条逐页用例内断言 | `src/shell/headerContract.dom.test.jsx` |
| 理由注册表键集 == HEADERLESS_PAGES 且理由必须已知 | `declares a known reason for exactly the headerless pages` | `src/shell/headerContract.dom.test.jsx` |
| 非 headerless 页必须有 PAGE_META 标题；headerless 页必须不在 PAGE_META | `gives every non-headerless page a PAGE_META title` | `src/shell/headerContract.dom.test.jsx` |
| PageShell 对任意 PAGE_META 键渲染页头（改为不锚定具体键，取 `Object.keys(PAGE_META)[0]`） | `renders the PAGE_META header for a header-bearing page`（改写） | `src/shell/headerContract.dom.test.jsx` |
| `PANELS` 键集恰等于 IA 全页面集且都是组件 | `maps every page key to a panel component and nothing else` | `src/shell/headerContract.dom.test.jsx` |
| nav.js 集合一致性：PAGE_META(2) ∪ HEADERLESS(9) == ALL_PAGES(11) | `covers every page key exactly` | `src/nav.test.js` |
| nav.js 侧的理由登记（纯 node，保留上一轮新增） | `declares a headerless reason for every page missing from PAGE_META` | `src/nav.test.js` |
| PageShell 渲染 PAGE_META 标题+描述（project 已移出 PAGE_META，改用 progress） | `renders the PAGE_META title, description and children`（改写） | `src/shell/pageShell.dom.test.jsx` |
| 每个 headerless 页的 PageShell body 都无 heading | `renders every headerless page body without a page header` | `src/shell/pageShell.dom.test.jsx` |
| 冗余页头不回潮（fe00626e 锚定） | `renders the team row with captain, member agent tags and both row actions`、`renders both executions with type labels, node state tags and status tags`、`keeps execution browsing focused on filtering and refresh` | `src/team.dom.test.jsx`、`src/fleet/fleet.dom.test.jsx` |

## 验证

- 定向：`npx vitest run src/shell src/nav.test.js src/team.dom.test.jsx src/fleet/fleet.dom.test.jsx src/project src/dag src/agentsConfig.dom.test.jsx src/app.dom.test.jsx src/todoPanel.dom.test.jsx src/chat.dom.test.jsx` → **27 files / 256 tests 全绿**（此时 chat 相关文件尚未被并发 agent 改动）。
- 全量：`npx vitest run`（本轮全部改动在位，15:10 起）→ **103 files / 737 tests 全绿，0 failed**（95s）。此前一次全量（14:54）曾出现 15 个失败，全部位于 `src/chat.dom.test.jsx`(12)、`src/sidebar.dom.test.jsx`(2)、`src/harness/management.dom.test.jsx`(1)，由并发 agent 当时正在落盘的在制品（`src/chat.jsx`、`src/chatSidebar.jsx`、`src/harness/management.jsx`、`src/operators/*`）造成；这三个文件均不 import `nav.js` / `shell/*`，对方改动落定后复跑即全绿。
- 反向探针 a（死配置方向）：向 `HEADERLESS_REASONS` 注入 `progress: 'body-title'` 并从 `PAGE_META` 删除 progress → `progress keeps exactly one title source` 失败（`expected null to be truthy`：progress 面板 body 无 Tabs/heading，理由不成立）。探针后原子还原。
- 反向探针 b（静默改理由方向）：把 `project` 的理由从 `body-title` 改为 `menu-only` → `project keeps exactly one title source` 失败（`expected <div role="tablist"> to be null`，由本轮新增的互斥断言抓住；若无互斥断言该方向会静默通过）。探针后原子还原，`git diff crates/web/spa/src/nav.js` 只剩本轮预期改动。
- 构建校验：`npx vite build --outDir /tmp/spa-verify-dist --emptyOutDir` → 7285 modules transformed，产物正常生成。刻意输出到 /tmp 而非 `spa/dist`，避免与并发 agent 正在进行的 dist 重建互相覆盖。
- `crates/web/spa/dist` 本轮**未**重建提交（并发 agent 正在重建 dist）；落地发布前需由发布流程 `npm run build` + `scripts/check-spa-drift.sh` 统一重建。

稳定文档已同步：[Web 模块](../../../agents/web/index.md)（`HEADERLESS_REASONS` 9 页两类理由 + `PAGE_META` 只留 progress/ownerview + project/dag/todos/chat 未挂 PageShell、死导入已删的说明）。

> 勘误（后续迭代补正）：本条目与 `agents/web/index.md` 当时都只说了「progress/ownerview 挂 PageShell」，漏了另外 6 个无标题挂载点（fleet/teams、fleet/nodes、fleet/executions、agentsConfig、brain/workbench、envs/todoPanel）。三段式事实（8 挂载点 / 2 渲染页头 / 6 惰性包装）与随后补的三个 P2（`pageShell.jsx` 门面注释、`bare` 空转单测、menu-only 的移动端 Select 断言）见 [spa-page-shell-mount-truth.md](spa-page-shell-mount-truth.md)。

> 勘误（后续迭代补正）：本条把 `project` 登记为 `body-title`、删掉 `project/project.jsx` 的 PageShell 死导入，`项目` 页从此没有页头；用户随后要求找回该页头，project 已回到 `PAGE_META` 并真挂 `<PageShell page="project">`（`HEADERLESS_REASONS` 由 9 页变 8 页），fe00626e 对 menu-only 页「不加冗余页头」的判断不受影响。另：本条 Commit 行原写 `dc321948 (working-tree)`——dc321948 只是当时的 HEAD，本条改动真正落在 66295a68，现已更正。详见 [spa-project-page-header-restored.md](spa-project-page-header-restored.md)。
