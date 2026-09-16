Commit: 5a7722cf

# SPA 页头机制收尾：PageShell 挂载面如实登记（8/2/6）、`bare` 空转单测锚回活配置、menu-only 的移动端一半补守卫（零 UI 变化）

## 背景

上一轮 `spa-headerless-reason-contract.md`（66295a68）把页头注册表改成说真话并逐面板挂了契约，但评审复查发现四处收尾没做干净，全部是纯 SPA 源码/文档层面、零 UI 影响：

1. **P1 模块记忆不准**：`agents/web/index.md` 与该 changelog 都写成「只有 progress/ownerview 挂 PageShell」，把「渲染页头的挂载点」当成「挂载点」。实测 `grep -rn PageShell src`：**8 个挂载点**，其中 2 个渲染页头（`project/progressPanel.jsx` `progress`、`project/ownerViewPanel.jsx` `ownerview`，正好是 `PAGE_META` 全集），6 个是无标题包装（`fleet/teams.jsx` team、`fleet/nodes.jsx` nodes、`fleet/executions.jsx` topics、`agentsConfig.jsx` agents、`brain/workbench/index.jsx` brain 只取 `.oc-page` body；`envs/todoPanel.jsx` 只取 `extra` 动作行）。
2. **P2 门面注释陈旧**：`shell/pageShell.jsx:1` 仍自称「the unified page header for **every** menu page」，与「11 页里只有 2 页有页头」的现实矛盾。
3. **P2 单测空转**：`pageShell.dom.test.jsx` 的 `bare` 用例锚在 `page="nodes"`——`nodes` 不在 `PAGE_META`（改动前后都不在），所以即使 `bare` 被整个忽略，「无 heading」也照样成立，用例等于没测；`bare` 同时没有任何生产调用方，死代码被绿色测试掩护。
4. **P2 menu-only 理由只守了一半**：理由文案说页名由「侧栏 Menu **与移动端 Select 标签**」承担，但契约只校验 `NAV_CATEGORIES` 的文案非空（`headerContract.dom.test.jsx` 挂载的是面板，不是 `<App/>`），`main.jsx:211` 那个窄栏 Select 没有任何断言（`app.dom.test.jsx` 原先只断言窄栏下 `.fleet-desktop-nav` 为 null）。删掉 Select 或让它不带标签，4 个 menu-only 页在移动端会彻底没有页名，而全套测试依旧全绿。

另外顺手清掉一个读代码就会困惑的伪 key：`envs/todoPanel.jsx` 的 `<PageShell page="todo">`——IA 里没有 `todo` 页（应为 `todos`），且该子面板是 `todos` 页里的一个 tab，本来就不该有自己的页标题。

## 决策与改动

- **不改 UI，不动注册表**：`nav.js` 的 `HEADERLESS_REASONS` / `PAGE_META` 一字未改（契约测试与浏览器验收脚本的锚点因此不受影响），本轮只让文档、注释与测试追上 66295a68 建立的机制。
- `shell/pageShell.jsx`：门面注释改写为「服务 `PAGE_META` 页；无条目的 page key 只渲染 `.oc-page` body（传了 `extra` 就多一行动作行）」，并逐个列出 8 个挂载点及其性质；`bare` 的 prop 文档如实写明「今日无生产调用方，保留为逃生口，并由单测锚在真实 `PAGE_META` 页上防止腐化」。
- `shell/pageShell.dom.test.jsx`：
  - `bare` 用例锚点从 `nodes` 改为 `progress`，并在同一用例里先证明「不传 `bare` 时该页确实渲染出 `进展` 标题」再证明「传了就消失」，两侧都咬合；
  - 新增 `extra`-only 用例（不传 `page`）：断言 `.oc-page-header` 存在、`.oc-page-title` 为 null、动作按钮在——这正是 `envs/todoPanel.jsx` 依赖的形状。
- `envs/todoPanel.jsx`：删掉伪 key `page="todo"`，只留 `extra`，并在 `return` 上方写明「不传 page 是有意的：将来 `PAGE_META` 收了 `todos` 也不会在这个子面板里冒出页标题」。DOM 结构逐字不变（`hasHeader` 原本就只由 `extra` 撑起来），`todoPanel.dom.test.jsx` + `envs/` 11 项全绿。
- `app.dom.test.jsx`：新增 `names every menu-only page in the mobile page Select`——从 `HEADERLESS_REASONS` 派生 menu-only 页集（当前 4 页：topics / team / chat / nodes），逐页 `setState({ page })` + 渲染真实 `<App/>`，断言窄栏 Select 存在且选中标签逐字等于该页的 nav 文案。选择器同时接受 antd 6 的 `.ant-select-content` 与 antd 5 的 `.ant-select-selection-item`，避免一次类名重命名就让守卫失效。
- `shell/headerContract.dom.test.jsx`：两类理由的互斥断言加自定义失败信息（原来失败只报 `expected <h5>… to be null`，维护者看不出该改哪边）；`bodyTitleOf` 与 `menuLabelOf` 的注释写明「接受任意 heading 是有意比文案更宽」以及「移动端 Select 的 DOM 一半在 app.dom.test.jsx」。
- 文档：`agents/web/index.md` 的 PageShell 条目改写为三段式事实（8 挂载点 / 2 渲染页头 / 6 惰性包装，含 `envs/todoPanel.jsx` 伪 key 的处置），并在 `spa-headerless-reason-contract.md` 原文两处就地加勘误指向本条。

## 未做（有意）

- **`spa/dist` 仍未重建**：共享工作区含并发 agent 的在制品，在其上重建会把别人的半成品烤进产物，故 dist 与 `scripts/check-spa-drift.sh` 一并留给发布前统一执行；本轮 5 个改动文件逐个过了 `npx esbuild --loader:.jsx=jsx --jsx=automatic` 语法校验。（本轮 15:57 在共享树尝试 `npx vite build` 时确实失败于未跟踪在制品 `src/agents/resourceTab.jsx` 的 JSX 语法错误；该文件随后已被其作者补全，这只是一次带时间戳的观测，不代表当前状态。）
- cargo 三件套（build/test/clippy）未跑：本轮零 Rust 改动（`git diff --name-only` 全在 `crates/web/spa/src` 与 md 文档），且工作区有并发 agent 正在改 `crates/agents`、`crates/web/src`、`crates/control`，此刻的 workspace 结果不可归因。
- 6 个惰性 PageShell 包装的清理（是否收成一个显式的 `PageBody`/`ActionRow`）另开迭代：`brain/workbench/`、`fleet/`、`agentsConfig.jsx` 正在其他 agent 手上。
- `项目` 页要不要恢复 H1 标题属需求层未决项：本轮维持 66295a68 的结论（不回退 fe00626e 的「页头冗余」判断），若要标题回来，接线点是 `project/project.jsx` 包一层 `<PageShell page="project">` + `nav.js` 把 project 移出 `HEADERLESS_REASONS` 并补回 `PAGE_META` 条目（契约测试读注册表，自动适配）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| `bare` 真的能跳过 PAGE_META 页的页头（两侧咬合，不再空转） | `bare skips a header the same PAGE_META page would otherwise render`（改写） | `src/shell/pageShell.dom.test.jsx` |
| 只传 `extra` 时渲染动作行且不渲染标题（envs 子面板依赖的形状） | `renders an extra-only header row (no title) outside PAGE_META`（新增） | `src/shell/pageShell.dom.test.jsx` |
| PAGE_META 标题+描述、headerless 页无 heading、未知 key 无页头 | `renders the PAGE_META title, description and children` 等 5 项（未改） | `src/shell/pageShell.dom.test.jsx` |
| 每个 menu-only 页在窄栏 Select 上都看得到页名 | `names every menu-only page in the mobile page Select`（新增） | `src/app.dom.test.jsx` |
| 理由互斥 + 每页标题来源唯一（失败信息改为可操作） | `every page keeps exactly one title source`（11 页逐个） | `src/shell/headerContract.dom.test.jsx` |
| `envs/todoPanel.jsx` 去掉伪 key 后行为不变 | `TodoPanel 模板 tab` / `TodoEnvsPanel` 相关 11 项（未改） | `src/todoPanel.dom.test.jsx`、`src/envs/toolsDrawer.dom.test.jsx` |

## 验证

- 全量 SPA：`npx vitest run` → **103 files / 739 tests 全绿，0 failed**（91.5s；上一轮基线 103/737，本轮净增 2 项 = `extra`-only + 移动端 Select）。
- 定向：`npx vitest run src/app.dom.test.jsx src/shell/` → **3 files / 45 tests 全绿**；`npx vitest run src/todoPanel.dom.test.jsx src/envs/` → **2 files / 11 tests 全绿**。
- 反向探针 A（移动端一半）：从 `main.jsx` 删掉窄栏 `<Select className="fleet-mobile-nav" …>` → 新增用例失败 `mobile page Select missing while on topics: expected null to be truthy`。探针后按 /tmp 备份原样还原，`git diff crates/web/spa/src/main.jsx` 为空。
- 反向探针 B（`bare` 空转）：删掉 `pageShell.jsx` 的 `if (bare) return <>{children}</>` 分支 → `bare skips a header the same PAGE_META page would otherwise render` 失败（改写前该方向的用例锚在 `nodes`，删分支后依旧全绿，即空转得证）。探针后原样还原。
- 语法/构建：5 个改动文件逐个 `npx esbuild --loader:.jsx=jsx --jsx=automatic <file> --outfile=/dev/null` → 全部 OK。整包 `npx vite build` 本轮**没有取得正面证据**：15:57 在共享树尝试时失败于并发在制品 `src/agents/resourceTab.jsx` 的 JSX 语法错误（与本轮无关，本轮文件均不在报错路径上），此后未复跑。

稳定文档已同步：[Web 模块](../../../agents/web/index.md)（PageShell 8 挂载点 / 2 渲染页头 / 6 惰性包装的三段式事实 + menu-only 移动端守卫的落点），并对 [上一轮 changelog](spa-headerless-reason-contract.md) 就地加了勘误。

> 勘误（后续迭代补正）：本条记录的「8 挂载点 / 2 渲染页头 / 6 惰性包装」是 5a7722cf 当时的事实。随后 `项目` 页按用户诉求恢复页头——`project/project.jsx` 真挂 `<PageShell page="project">`、`project` 移出 `HEADERLESS_REASONS` 并回到 `PAGE_META`，渲染页头的挂载点即 `PAGE_META` 全集 project / progress / ownerview；`pageShell.jsx` 与 `agents/web/index.md` 的措辞同时去掉硬编码计数，改以 `grep -rn '<PageShell' src` 为权威名单。本条 Commit 行（原写「working-tree, 待提交」→ 5a7722cf）与上面两处「vite build 当前失败」的瞬时断言也在同一 docs-only 提交里更正。详见 [spa-project-page-header-restored.md](spa-project-page-header-restored.md)。
