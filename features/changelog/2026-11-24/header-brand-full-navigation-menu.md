# 顶栏品牌词标成为完整导航入口

一句话：Header 的「⛵ Opencoder Fleet」从静态词标变为导航按钮——hover / 点击打开一个
按分类分组的完整 IA 菜单，选择任一页面即关闭面板并走既有的显式 `goPage`；Sider 不再
在顶部渲染分类 `CategoryTabs`，只保留当前分类的页面 Menu，移动端仍为分类 tabs +
Select 两级。

## 行为与实现

- `shell/brandNav.jsx`：`BrandNavMenu` 用 antd `Popover`（`trigger=['hover','click']`）
  承载 `Menu mode="inline"`，条目来自 `nav.js::groupedMenuItemsOf(categories)`——
  每个可见分类一组（组标题 = `category.label`），子项复用 `menuItemsOf`；权限过滤
  后的 `visible` 由调用方传入，组件自身只持有 open/closed 状态，选择后先关闭再
  `onNavigate(page)`。品牌触发按钮带 `aria-haspopup="menu"` / `aria-expanded`，
  文本与点击热区保持不变。
- `main.jsx`：Header 以 `<BrandNavMenu categories={visible} selectedPage={shownPage}
  onNavigate={goPage} />` 替换静态 `<span className="fleet-brand">`；移除 Sider 顶部
  的 `.fleet-nav-category` / `CategoryTabs`，`CategoryTabs` 仍用于移动端。
- `app.css`：`.fleet-brand` 改为按钮样式（hover / `aria-expanded="true"` 背景、
  `focus-visible` 外框、caret），新增 `.fleet-brand-popover` / `.fleet-nav-panel`
  的尺寸与滚动规则，删除折叠态 `.fleet-nav-category` 隐藏规则。
- 验收脚本（`scripts/acceptance/`）：桌面端不再从 `.fleet-nav-category` 切分类，
  改为 hover `.fleet-brand` 后在 `.fleet-nav-panel` 内选 `menuitem`；
  `spa_responsive.js` 桌面端分类标签取自 `NAV_CATEGORIES`，分类切换只在 <768 的
  tabs 上执行。
- `nav.js`：`ontologyEntities` 的标题来源从 `menu-only` 修正为 `body-title`，
  与实体页实际渲染的 Tabs 对齐；这是既有登记漂移，由 `headerContract` 指出。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 词标渲染为菜单触发按钮 | `renders the wordmark as a menu trigger button` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 打开完整 IA（全部分组与页面） | `opens the complete IA on hover: every category group and page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 点击打开、选择后关闭 | `opens on click (touch/keyboard) and closes after choosing a page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 面板导航后关闭 | `closes a hover-opened panel after choosing a page from the full IA` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 遵循权限过滤后的分类列表 | `honors the permission-filtered category list it is handed` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 分组投影按序复用 `menuItemsOf` 子项 | `groups the complete IA by category in order, reusing menuItemsOf children` | `crates/web/spa/src/nav.test.js` |
| 只分组传入的分类（权限过滤 IA） | `groups only the categories handed in (permission-filtered IA)` | `crates/web/spa/src/nav.test.js` |
| 壳层经品牌菜单导航并重定分类 | `navigates through the header brand menu and lands on the selected page`、`scopes the project category to Overview and the standalone Tag page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 移动端分类 tabs + Select 保留 | `drops the retired desktop Segmented nav from the Content top`、`names %s in the mobile page Select and sidebar` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 显式导航持久化到 localStorage | `首次进入不预写存储；显式导航把所选页写入 localStorage` | `crates/web/spa/src/navPersistence.dom.test.jsx` |
| 项目深链与菜单导航 | `opens a shared project ahead of a saved page, follows history and clears the link on menu navigation` | `crates/web/spa/src/appProjectLinks.dom.test.jsx` |
| 标题来源登记与实体页一致 | `ontologyEntities keeps exactly one title source` | `crates/web/spa/src/shell/headerContract.dom.test.jsx` |

## 验证

- `npm ci` 恢复二维图谱依赖后，定向 Vitest：`src/nav.test.js`、
  `src/appNavigation.dom.test.jsx`、`src/navPersistence.dom.test.jsx`、
  `src/appProjectLinks.dom.test.jsx`、`src/shell/brandNav.dom.test.jsx`、
  `src/shell/categoryTabs.dom.test.jsx` 共 6 个文件、54 个用例通过；
  `src/shell/headerContract.dom.test.jsx` 21 个用例通过。
- 验收脚本逐个 `node --check` 通过；`rg "fleet-nav-category" scripts/acceptance`
  无结果。
- 未运行全量 SPA 测试、浏览器验收与 Rust 测试；构建与 dist 产物核验见本次父流程。
- 收尾复跑：定向 Vitest 23 个文件 187/187（图谱、shell、nav、appNavigation / appProjectLinks / navPersistence、fleet/navigation）；`npm run typecheck:ontology` 无错误；`build-spa.sh` 重建后 `check-spa-drift.sh` 报 `spa dist: no drift`。浏览器验收仍未运行。
