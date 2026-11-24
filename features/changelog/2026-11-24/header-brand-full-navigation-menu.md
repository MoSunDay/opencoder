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
  `onNavigate(page)`。
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

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 词标渲染为菜单触发按钮 | `renders the wordmark as a menu trigger button` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 打开完整 IA（全部分组与页面） | `opens the complete IA on hover: every category group and page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 点击打开、选择后关闭 | `opens on click (touch/keyboard) and closes after choosing a page` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| hover 面板导航后关闭且页面键完整 | `closes the hover-opened panel after navigating and keeps the page key intact` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 遵循权限过滤后的分类列表 | `honors the permission-filtered category list it is handed` | `crates/web/spa/src/shell/brandNav.dom.test.jsx` |
| 分组投影按序复用 `menuItemsOf` 子项 | `groups the complete IA by category in order, reusing menuItemsOf children` | `crates/web/spa/src/nav.test.js` |
| 只分组传入的分类（权限过滤 IA） | `groups only the categories handed in (permission-filtered IA)` | `crates/web/spa/src/nav.test.js` |
| 默认页渲染品牌触发器与当前分类菜单 | `renders the brand trigger and the node-category menu on the default page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 经头部品牌菜单导航并落在所选页 | `navigates through the header brand menu and lands on the selected page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 显式导航持久化到 localStorage | `首次进入不预写存储；显式导航把所选页写入 localStorage` | `crates/web/spa/src/navPersistence.dom.test.jsx` |
| 项目深链与菜单导航 | `opens a shared project ahead of a saved page, follows history and clears the link on menu navigation` | `crates/web/spa/src/appProjectLinks.dom.test.jsx` |

- 用户授权免测提交：本次提交未运行 Vitest / 验收脚本 / 构建，本记录不声称上述测试已通过。
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。
