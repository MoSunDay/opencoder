# 顶栏品牌词标成为完整导航入口

一句话：Header 的「⛵ Opencoder Fleet」从静态词标变为导航按钮——hover / 点击打开一个
按分类分组的完整 IA 菜单，选择任一页面即关闭面板并走既有的显式 `goPage`；Sider 不再
在顶部渲染分类 `CategoryTabs`，只保留当前分类的页面 Menu，移动端仍为分类 tabs +
Select 两级。

## 行为与实现

- `shell/brandNav.jsx`：`BrandNavMenu` 用 antd `Popover`（桌面 `trigger=['hover','click']`；
  `pointer: coarse` 只绑定 click，避免触屏 tap 的 mouseenter + click 闪开闪关）承载
  `Menu mode="inline"`，条目来自 `nav.js::groupedMenuItemsOf(categories)`——每个可见
  分类一组（组标题 = `category.label`），子项复用 `menuItemsOf`；权限过滤后的 `visible`
  由调用方传入，组件自身只持有 open/closed 状态，选择后先关闭、把焦点还给品牌按钮，
  再 `onNavigate(page)`。品牌触发按钮带 `aria-haspopup="menu"` / `aria-expanded` 与
  打开时的 `aria-controls`；可访问名包含可见词标（`Opencoder Fleet：打开完整导航`）。
  键盘/AT 打开（Enter/Space/ArrowDown）后焦点进入首个菜单项，hover 不抢焦点，Escape
  关闭并回到品牌按钮；当前页菜单项带 `aria-current="page"`。
- `main.jsx`：Header 以 `<BrandNavMenu categories={visible} selectedPage={shownPage}
  onNavigate={goPage} />` 替换静态 `<span className="fleet-brand">`；移除 Sider 顶部
  的 `.fleet-nav-category` / `CategoryTabs`，`CategoryTabs` 仍用于移动端。Sider Menu
  增加分类级可访问名（如 `后台管理页面导航`）与当前页 `aria-current="page"`。
- `app.css`：`.fleet-brand` 改为按钮样式（hover / `aria-expanded="true"` 背景、
  `focus-visible` 外框、caret），新增 `.fleet-brand-popover` / `.fleet-nav-panel`
  的尺寸与滚动规则，删除折叠态 `.fleet-nav-category` 隐藏规则。
- 验收脚本（`scripts/acceptance/`）：桌面端一律 hover `.fleet-brand` 后在
  `.fleet-nav-panel` 内按精确名称选 `menuitem`；收尾补齐 `dag_results.js`、
  `dag_dynamic.js`、`todo_workbench/editor.js`、`brain/runtime.js`、
  `brain/layered-panels.js`、`brain/label-boundary.js` 中旧的分类 tablist / 全页
  `menuitem` 写法。`spa_responsive.js` 桌面端分类标签取自 `NAV_CATEGORIES`，分类
  切换只在 <768 的 tabs 上执行。
- `nav.js`：`menuItemsOf` 给装饰性页面图标加 `aria-hidden`，菜单项的可访问名
  只含页面文案（Sider 与品牌菜单一致，精确名称查询不再被图标名污染）；
  `withCurrentPage` 以纯函数递归标注当前页 `aria-current="page"`，品牌分组投影与
  Sider 扁平投影共用；`ontologyEntities` 的标题来源从 `menu-only` 修正为
  `body-title`，与实体页实际渲染的 Tabs 对齐——这是既有登记漂移，由
  `headerContract` 指出。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 词标渲染为菜单触发按钮 | `renders the wordmark as a menu trigger button` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 打开完整 IA（全部分组与页面） | `opens the complete IA on hover: every category group and page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 点击打开、选择后关闭 | `opens on click (touch/keyboard) and closes after choosing a page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 面板导航后关闭 | `closes a hover-opened panel after choosing a page from the full IA` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 遵循权限过滤后的分类列表 | `honors the permission-filtered category list it is handed` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 键盘打开焦点进入首项、Escape 回焦 | `moves focus into the first menu item on keyboard open and restores it on Escape` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 触发按钮关联菜单并标出当前页 | `links the trigger to the menu and marks the current page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 粗指针只响应 click | `opens on click but not on hover when the pointer is coarse` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| Sider 菜单可访问名与当前页 | `renders the brand trigger and the node-category menu on the default page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 分组投影按序复用 `menuItemsOf` 子项 | `groups the complete IA by category in order, reusing menuItemsOf children` | `crates/web/spa/src/nav.test.js` |
| 当前页标记纯函数 | `marks only the selected leaf in the grouped brand projection`、`marks a flat Sider projection and leaves unselected or unknown pages unmarked` | `crates/web/spa/src/nav.test.js` |
| 只分组传入的分类（权限过滤 IA） | `groups only the categories handed in (permission-filtered IA)` | `crates/web/spa/src/nav.test.js` |
| 壳层经品牌菜单导航并重定分类 | `navigates through the header brand menu and lands on the selected page`、`scopes the project category to Overview and the standalone Tag page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 移动端分类 tabs + Select 保留 | `drops the retired desktop Segmented nav from the Content top`、`names %s in the mobile page Select and sidebar` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 显式导航持久化到 localStorage | `首次进入不预写存储；显式导航把所选页写入 localStorage` | `crates/web/spa/src/navPersistence.dom.test.jsx` |
| 项目深链与菜单导航 | `opens a shared project ahead of a saved page, follows history and clears the link on menu navigation` | `crates/web/spa/src/appProjectLinks.dom.test.jsx` |
| 标题来源登记与实体页一致 | `ontologyEntities keeps exactly one title source` | `crates/web/spa/src/shell/headerContract.dom.test.jsx` |

## 验证

- `npm ci` 恢复二维图谱依赖后，定向 Vitest 7 个文件、75 个用例通过：
  `src/nav.test.js`、`src/appNavigation.dom.test.jsx`、
  `src/navPersistence.dom.test.jsx`、`src/appProjectLinks.dom.test.jsx`、
  `src/shell/brandNav.dom.test.jsx`、`src/shell/categoryTabs.dom.test.jsx`、
  `src/shell/headerContract.dom.test.jsx`。
- 验收脚本逐个 `node --check` 通过；`rg "fleet-nav-category" scripts/acceptance`
  无结果。
- 本次收尾 `npm run build` 通过并重建 `crates/web/spa/dist/static/app.js`
  （4,657,018 字节）与 `app.css`（68,113 字节）；`scripts/check-spa-drift.sh`
  报 `spa dist: no drift`。
- 此前真实 Chromium 冒烟（1440×900 与 390×844）确认品牌 hover/click 菜单、
  桌面 Sider 无分类 tablist、移动端分类 tabs 保留、菜单项切页与 Ontology
  二维图谱均正常，无 console/page error，桌面/移动端无横向溢出。
- 未运行全量 SPA 测试、完整浏览器验收套件与 Rust 测试。
- 收尾复跑：定向 Vitest 23 个文件 187/187（图谱、shell、nav、appNavigation / appProjectLinks / navPersistence、fleet/navigation）；`npm run typecheck:ontology` 无错误；`build-spa.sh` 重建后 `check-spa-drift.sh` 报 `spa dist: no drift`；完整浏览器验收套件仍未运行。
- 收尾 a11y 与剩余验收脚本：定向 Vitest 6 个文件 78/78（`nav`、`brandNav`、`appNavigation`、`navPersistence`、`appProjectLinks`、`headerContract`）；6 个改动验收脚本 `node --check` 通过；`rg` 审计确认无 `fleet-nav-category`、无 `getByText('Agent')` 页面导航、无 `.fleet-sidebar [role=tablist]` 正向用法（仅 `ontology/browser.mjs` 负向断言）；移动端 `导航分类` tablist 保留。
- 已知限制：antd 6.6.2 `Popover` 容器仍由 rc-tooltip 渲染为 `role="tooltip"` 并挂 `aria-describedby`；本次通过 `aria-controls`、焦点管理、可访问名对齐与 `aria-current` 降低影响，严格 menu 语义需另行替换浮层实现。
- 屏宽验收脚本：先补齐 5 个既有 fixture 缺口（`/api/project/todos/nfs`、`/api/ontology/envs/debug/directories/tree`、`freshness/policies|runs`，响应形状取自 `ontology/api.ts` 与 `AgentNfsCard.jsx`；此前 fixture 停在 2026-10-19，脚本因 fixture 服务对未登记请求的守卫以 exit 2 收尾）。随后 `CHROME_PATH=<系统 Chrome> node scripts/acceptance/spa_responsive.js --width {1920,1280,768,390} --port 0 --require-committed` 四个宽度均 exit 0：各访问 17 页，布局测量 43/44/44/44 次 0 溢出，桌面端分类切换经品牌菜单 hover → `.fleet-nav-panel` 完成；截图与日志在 `/tmp/uitest/wrapup-<宽度>`。
- Ontology 真实链路：`python3 scripts/acceptance/ontology/main.py --server target/release/opencoder-server --root <仓库外新目录>` 通过（私有挂载命名空间内启动 Server 并真实只读 NFS 挂载，回执 `nfs_read_only: true`；`browser.mjs` → `experience.mjs` 覆盖 4 页无标题、属性与 Action 配置、实体与关系创建、切面保存、草稿守卫、详情标签、二维图谱控制与观测恢复，桌面端品牌面板列出全部 17 个页面，240px 移动端分类 tabs 可横向滚动）。回执 `/tmp/oc-acceptance-20261124-222742/result.json`。
- Ontology 能力目录：`python3 scripts/acceptance/ontology/catalog.py --server … --root <仓库外新目录>` 通过：4 宽度 × 4 个详情标签 16 次测量、实体列表 4 宽度（390 另含创建、加载、错误、重试）9 次测量 0 溢出，生成字段只读、补充说明同步后保留，内嵌 SPA 与 `crates/web/spa/dist` 逐文件一致；回执 `/tmp/oc-catalog-20261124-222920/receipt.json`。
- 验收脚本同步（随本改动及既有漂移）：`ontology/browser.mjs`、`catalog.mjs`、`entities.mjs` 桌面端改经品牌面板切换分类（原依赖已移除的 Sider 分类 tablist）；`browser.mjs` 侧栏点击限定 `#fleet-page-menu`，图谱页待元数据与切面加载后先切到「自定义观测」再开筛选（Server 启动即同步内置切面，页面默认恢复到「切面观测」），实体列表经搜索框定位（Server 同步的能力实体使 20 行表格分页）；`experience.mjs` 图谱定位只取可见 viewport；`main.py`、`catalog.py` 启动 Server 改传 `--web true`（`--web` 已是显式布尔参数）。
- 完整浏览器验收套件（`scripts/acceptance/ui/main.js`，需成套二进制与配套镜像）仍未运行。
