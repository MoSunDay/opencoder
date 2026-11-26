# 顶栏品牌菜单改为一级 tab + 图标

一句话：Header「⛵ Opencoder Fleet」悬停 / 点击打开的面板不再列出每个分类下的全部页面，
只列四个一级 tab——项目 / Agent / Ontology / 后台管理，各带分类图标；选择一个 tab 进入该
分类首页（`categoryHome`），页面列表仍由桌面 Sider 与 <768 的分类 tabs + 「页面导航」Select
承载。

## 背景与决策

- 用户反馈：品牌悬停菜单里仍然带着各 tab 下的子项，需要的是「一级 tab + 图标」。此前
  `43d373697` 只给分组标题补了图标，分组下的页面项原样保留，面板依旧是完整 IA。
- 决策：品牌菜单只承担分类级（一级）导航，页面级导航交给 Sider / 移动端 Select；不新增
  导航状态，当前分类仍纯由 store `page` 推导。

## 行为与实现

- `crates/web/spa/src/nav.js`：删除 `groupedMenuItemsOf`，新增 `categoryMenuItemsOf(categories)`
  ——每个分类一个扁平 `{ key, label, icon }`（图标 `aria-hidden`，可访问名仍是分类名）；新增
  `categoryKeyOfPage(categories, page)`（所属分类 key，未知页面返回 `undefined`）；
  `withCurrentPage` 简化为扁平 map（Sider 与品牌菜单两个调用方都是扁平列表），仍是纯函数。
- `shell/brandNav.jsx`：`activeCategory = categoryKeyOfPage(categories, selectedPage)`，
  `selectedKeys=[activeCategory]`，点击 tab 走 `chooseCategory(key)` → 关闭面板、焦点回到
  词标、`onNavigate(categoryHome(key))`。面板 `aria-label="一级导航"`，按钮
  `aria-label="Opencoder Fleet：打开一级导航"`；键盘 / 焦点 / Escape / 粗指针逻辑不变，当前
  分类 tab 带 `aria-current="page"`。
- `app.css`：`.fleet-nav-panel` 宽 `min(200px, calc(100vw - 48px))`（与 200px Sider 对齐），
  删除 `max-height / overflow-y / overscroll-behavior`（四项不会溢出）与三条分组标题规则，
  新增 `.fleet-nav-panel .ant-menu-item .anticon { font-size: 16px; }`；注释同步。
- `main.jsx`、`shell/headerContract.dom.test.jsx`：仅头注释 / 注释措辞同步。
- `dist/static/app.{js,css}`：`scripts/build-spa.sh` 重建，`check-spa-drift.sh` 报
  `spa dist: no drift`。

## 验收脚本迁移（`scripts/acceptance/`）

- 新增 `harness/navigation.js`：`openPage(page, name)` 按用户实际路径走两步——桌面
  hover 品牌 → 点分类 tab → 在 `#fleet-page-menu` 点页面；<768 点分类 tab → 「页面导航」
  Select 选页面。分类归属直接 `import()` SPA `nav.js` 的 `NAV_CATEGORIES`，不另抄一份 IA。
  hover 3 s 内未打开面板时回退为点击品牌（鼠标未离开触发器时 rc-trigger 不会再次
  mouseenter，实测 `DAG 工作流` 一步 6.5 s 全部耗在回退等待上）。
- `harness/navigation.test.js`（node:test，9 例）：分类解析（字符串 / 正则、`'Agent'`
  精确落到 chat 页而非分类名、未知名称先于触页抛错）、桌面两分支、窄屏分支、非超时错误
  透传、hover 失败回退。
- 23 个脚本改用 `openPage`（`project_workbench_ui`、`project_order_ui`、`project/{browser,forms}`、
  `project/capabilities/browser`、`spa_responsive`、`artifact_stream`、`brain/{label-boundary,
  layered-panels,runtime}`、`conversations/support`、`dag_dynamic`、`dag_results`、
  `harness/{codex,settings}`、`ontology/{browser,catalog,entities}`、`platform`、`t12_ui_verify`、
  `todo_initialization_ui`、`todo_workbench/editor`、`ui/scenarios/access`），全部 `node --check`
  通过。`ontology/browser.mjs` 顺带把断言翻成新契约：品牌面板含 `Ontology` 且不含图谱 /
  实体等页面名，Sider 含全部 Ontology 页面。`t12_ui_verify.js` 的 `'团队组队'` 改为早已
  改名的 `'Team 组队'`。`ui/scope.js` 未动（未增删页面）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 词标渲染为菜单触发按钮 | `renders the wordmark as a menu trigger button` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 打开一级 tab：恰好四个分类、无子页面 | `opens the first-level tabs on hover: exactly the four categories, no child pages` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 点击打开、进入分类首页并关闭 | `opens on click (touch/keyboard), navigates to the category home and closes`、`closes a hover-opened panel after choosing a tab and lands on that category home` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 权限过滤后的 tab 列表 | `honors the permission-filtered category list it is handed` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 键盘打开进入首项、Escape 回到词标 | `moves focus into the first menu item on keyboard open and restores it on Escape` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 触发按钮关联菜单并标出当前分类 tab | `links the trigger to the menu and marks the current category tab` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 粗指针只响应 click | `opens on click but not on hover when the pointer is coarse` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 一级 tab 投影：顺序、图标、只投影传入分类 | `lists exactly one flat tab per category in IA order, without child pages`、`decorates every tab with its category icon, marked decorative`、`lists only the categories handed in (permission-filtered IA)` | `crates/web/spa/src/nav.test.js` |
| 页面 → 分类 key | `resolves a page to the key of the category that owns it`、`returns undefined for an unknown page or a page outside the handed-in categories` | `crates/web/spa/src/nav.test.js` |
| 当前标记纯函数（扁平） | `marks only the selected tab in the flat brand projection`、`marks a flat Sider projection and leaves unselected or unknown pages unmarked` | `crates/web/spa/src/nav.test.js` |
| 壳层经品牌 tab 进入分类首页并重定 Sider | `navigates through the header brand menu to a category home and re-scopes the Sider`、`scopes the project category to Overview and the standalone Tag page`、`renders the brand trigger and the node-category menu on the default page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 显式导航持久化到 localStorage | `首次进入不预写存储；显式导航把所选页写入 localStorage` | `crates/web/spa/src/navPersistence.dom.test.jsx` |
| 项目深链与菜单导航 | `opens a shared project ahead of a saved page, follows history and clears the link on menu navigation` | `crates/web/spa/src/appProjectLinks.dom.test.jsx` |
| 验收 helper：分类归属解析 | `every page label resolves back to its own category, as a string and as a RegExp`、`'Agent' resolves to the Agent category through its chat page, never through the category label`、`'实体' resolves to ontology; unknown or partial labels resolve to nothing` | `scripts/acceptance/harness/navigation.test.js` |
| 验收 helper：桌面 / 窄屏路径与回退 | `openPage() walks brand → category tab → Sider page when the page is outside the active category`、`openPage() only clicks the Sider page when its category is already active`、`openCategory() falls back to clicking the brand when hovering does not open the panel`、`openPage() switches the category tab, then picks the page Select when the Sider is hidden` | `scripts/acceptance/harness/navigation.test.js` |
| 验收 helper：失败透传与未知名称 | `openPage() surfaces non-timeout failures and rejects unknown labels before touching the page`、`navCategories() loads NAV_CATEGORIES from the SPA source of truth once per process` | `scripts/acceptance/harness/navigation.test.js` |

## 实际执行

- 定向回归：`npx vitest run` `nav.test.js`、`shell/brandNav.dom.test.jsx`、`appNavigation.dom.test.jsx`、
  `navPersistence.dom.test.jsx`、`appProjectLinks.dom.test.jsx` → 5 files / 60 passed；
  `shell/headerContract.dom.test.jsx`（仅注释改动）→ 21 passed。
- 验收 helper：`node --test scripts/acceptance/harness/navigation.test.js` → 9 passed（在 nav.js
  改动之后复跑）。23 个迁移脚本 `node --check` 通过。
- 构建：`scripts/build-spa.sh && scripts/check-spa-drift.sh` → `spa dist: no drift`。
- 浏览器（`CHROME_PATH=/usr/bin/chromium-browser`，dist + `ui/responsive/services.js` 夹具 API，
  admin 身份）：
  - 临时探针（`/tmp`，不入库）：1440×900 面板 `menuitem` 恰为 `项目 / Agent / Ontology / 后台管理`，
    无分组、无滚动，面板 200×164 px、完全在视口内，四个图标 `aria-hidden="true"` 且 16px；
    默认页 `节点列表` 时 `后台管理` tab 带 `aria-current="page"`；点 `Ontology` → 面板关闭、
    Sider 变为图谱等五页、无页面标题；再开面板 `Ontology` 为当前 tab，Escape 关闭；`openPage`
    跨分类（DAG 工作流）/ 同分类（定时任务）/ 后台（节点列表）/ 回首页（Overview）均落到
    正确页面。390×844：点击品牌打开面板（粗指针），宽 200 ≤ 390−48，无溢出；点 `Ontology` →
    分类 tab 切到 Ontology；`openPage('实体类型')`、`openPage('DAG 工作流')` 经 Select 落页。
    页面错误 0。截图 `/tmp/uitest/brand-tabs/{desktop-1440-open,mobile-390-open,mobile-390-after}.png`。
  - `node scripts/acceptance/project_workbench_ui.js` → `{"result":"PASS", widths:[1920,1280,768,390],
    page_errors:[]}`。
  - `node scripts/acceptance/spa_responsive.js --width {1920,1280,768,390}` → 四个宽度 exit 0，
    各访问 17 页，布局测量 43/43/44/44 次 0 溢出；桌面分类切换走品牌面板点 tab。截图
    `/tmp/uitest/brand-tabs/responsive-<宽度>/`。
- 文档：`agents/frontend/web/index.md`、`agents/frontend/web/ontology.md`、
  `features/platform/ontology/index.md` 三处措辞改为一级 tab 导航；历史 changelog 不改。
- Clippy / Rust 测试：未修改 Rust 代码，不适用。
- 测试范围：本次修改及其直接影响链路；未运行全量 SPA 测试、完整浏览器验收套件与 Rust 测试。

## Impact Surface

- 可见行为：所有身份的 Header 品牌菜单（桌面 hover / click，<768 click）只显示一级 tab；
  子页面须从 Sider 或移动端 Select 进入；非 admin 仍看不到「后台管理」。
- 可访问名变更：面板 `完整导航` → `一级导航`，按钮 `Opencoder Fleet：打开完整导航` →
  `Opencoder Fleet：打开一级导航`；CSS 类 `fleet-nav-group-label` 与 `groupedMenuItemsOf`
  删除。仓库内依赖这些名称的测试与验收脚本已全部迁移；仓库外脚本需同步。
- 部署：dist 内嵌进 server 二进制，线上要看到效果需重建并重启 server；本次未改 API、CLI、
  Rust 代码与依赖。

## 发布构建与验收实例部署（提交后补记）

- 提交 `4ac4659dd` 后 `CARGO_TARGET_DIR=/tmp/oc-rel-target scripts/platform/release/build.sh --output
  /tmp/oc-rel-brand/dist/opencoder-platform-4ac4659dd` → manifest 与 7 个二进制校验通过，
  `0.1.1 (4ac4659dd)`、`git_dirty=false`、`spa_sha256=bd873f6a…465d3d`，兼容矩阵与 `43d373697`
  相同（protocol 1 / data_format 12）。构建日志里 `opencoder-core` 的 2 条 warning 为既有警告，
  本次未改 Rust。
- `scripts/acceptance/ontology/main.py --server <包>/bin/opencoder-server --root /tmp/oc-ontology-acc-4ac4659dd`
  （私有挂载命名空间 + 真实只读 NFS）→ `passed: true`、`nfs_read_only: true`，浏览器回执
  `desktop: ["项目","Agent","Ontology","后台管理"]`，4 页检查全部为 true，清理完成。
- 验收实例 8091：`install.sh --source <包>/bin/opencoder-server --dest target/release/opencoder-server
  --no-build --backup`，回滚点 `target/release/opencoder-server.bak.20261126223537`（`43d373697`）；
  22:35:41 重启 `opencoder-server-acceptance`，`/api/health` → `0.1.1 (4ac4659dd)`，已装二进制
  SHA256 与发布包一致；重启后 journal 零 ERROR/WARN。真实浏览器打开 8091：面板为
  `项目 / Agent / Ontology / 后台管理 [current]`，页面错误 0，截图
  `/tmp/uitest/brand-tabs/live-8091-open.png`。
- 18080 Fleet 仍为 `43d373697`（旧菜单），四单元 active；是否把本次 UI 改动发到 Fleet 待决定。
