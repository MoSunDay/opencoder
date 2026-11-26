// nav.js — the single source of truth for the fleet-console information
// architecture (IA): 项目 / Agent / Ontology / 后台管理, each owning
// an ordered page list. Three navigation surfaces derive from the same
// categories: the header brand hover menu lists ONLY the first-level tabs —
// one flat item per category (categoryMenuItemsOf), picking one lands on the
// category home — the Sider renders a Menu scoped to the pages of the active
// category, and mobile keeps scrollable category tabs + a page Select. The
// active category is PURELY derived from the store `page` (no extra global
// navigation state). `items` is the single source for every consumer below.

import { createElement } from 'react';
import {
  CheckSquareOutlined,
  ClockCircleOutlined,
  ClusterOutlined,
  DeploymentUnitOutlined,
  MessageOutlined,
  ProfileOutlined,
  ProjectOutlined,
  RobotOutlined,
  SettingOutlined,
  ApartmentOutlined,
  DatabaseOutlined,
  LinkOutlined,
  SafetyCertificateOutlined,
  TeamOutlined,
  TagsOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';

/// Category → ordered pages. `icon` stores the icon COMPONENT reference (not
/// a JSX element) so this file stays plain .js; `menu` doubles as the mobile
/// Select option label.
export const NAV_CATEGORIES = [
  {
    key: 'project',
    label: '项目',
    icon: ProjectOutlined,
    items: [
      { page: 'project', menu: 'Overview', icon: ProjectOutlined },
      { page: 'projectTags', menu: 'Tag', icon: TagsOutlined },
    ],
  },
  {
    key: 'agent',
    label: 'Agent',
    icon: RobotOutlined,
    items: [
      { page: 'brain', menu: '大脑调度', icon: ThunderboltOutlined },
      { page: 'topics', menu: '全部执行', icon: ProfileOutlined },
      { page: 'schedules', menu: '定时任务', icon: ClockCircleOutlined },
      { page: 'dag', menu: 'DAG 工作流', icon: DeploymentUnitOutlined },
      { page: 'todos', menu: 'TODO 管理', icon: CheckSquareOutlined },
      { page: 'team', menu: 'Team 组队', icon: TeamOutlined },
      { page: 'chat', menu: 'Agent', icon: MessageOutlined },
      { page: 'agents', menu: 'Agent 配置', icon: RobotOutlined },
    ],
  },
  {
    key: 'ontology',
    label: 'Ontology',
    icon: ApartmentOutlined,
    items: [
      { page: 'ontologyGraph', menu: '图谱', icon: ApartmentOutlined },
      { page: 'ontologyEntities', menu: '实体', icon: DatabaseOutlined },
      { page: 'ontologyTypes', menu: '实体类型', icon: ProfileOutlined },
      { page: 'ontologyRelationships', menu: '关系类型', icon: LinkOutlined },
      { page: 'ontologyFreshness', menu: '数据保鲜', icon: SafetyCertificateOutlined },
    ],
  },
  {
    key: 'admin',
    label: '后台管理',
    icon: SettingOutlined,
    items: [
      { page: 'nodes', menu: '节点列表', icon: ClusterOutlined },
      { page: 'users', menu: '用户权限', icon: TeamOutlined },
    ],
  },
];

/// Flat registry of every page key (menu order, derived from NAV_CATEGORIES so
/// the two lists can never drift). Doubles as the validity set for the
/// persisted navigation selection (main.jsx): a stale or foreign stored value
/// must fall back to the default page, never crash a renderer.
export const ALL_PAGES = NAV_CATEGORIES.flatMap((c) => c.items.map((i) => i.page));

/// Fallbacks mirror the pre-IA shell: unknown pages land on the node
/// category / the nodes page.
export const DEFAULT_CATEGORY = 'admin';
/// localStorage key mirroring the last explicitly chosen category and page
/// so a reload restores the selection. Read and
/// written exclusively through the usehooks-ts `useLocalStorage` hook in
/// main.jsx — no other module touches localStorage for navigation state.
export const NAV_STORAGE_KEY = 'oc_nav_page';

/// Pages with no PageShell header: they are absent from PAGE_META, so
/// pageShell.jsx renders only the bare `.oc-page` body. Skipping the header
/// also skips the in-content page title, so every entry must declare WHY the
/// page still has a name — shell/headerContract.dom.test.jsx mounts the real
/// panel of every page and fails on an unknown page, an unknown reason, or a
/// PAGE_META entry no panel renders:
///   - 'body-title': the panel renders its own title (antd Tabs) in the body;
///   - 'menu-only' : full-bleed / operational page — the sidebar Menu and the
///                    mobile Select label are deliberately the only title (a
///                    page header would just repeat them, see fe00626e).
export const HEADERLESS_REASONS = {
  project: 'body-title',
  projectTags: 'menu-only',
  brain: 'body-title',
  topics: 'menu-only',
  schedules: 'menu-only',
  dag: 'body-title',
  todos: 'body-title',
  team: 'body-title',
  chat: 'menu-only',
  agents: 'body-title',
  nodes: 'menu-only',
  users: 'menu-only',
  ontologyGraph: 'menu-only',
  ontologyEntities: 'body-title',
  ontologyTypes: 'menu-only',
  ontologyRelationships: 'menu-only',
  ontologyFreshness: 'body-title',
};

/// Per-page header copy for the pages whose panel actually mounts PageShell
/// with its own key: pageShell.jsx renders
/// title + desc from here. An entry no panel renders is dead copy — the
/// header contract test mounts every panel and fails on it, so this map
/// stays truthful.
export const PAGE_META = {};

/// Category lookup with the default as the safety net (unknown keys never
/// crash a renderer — they just show the default category).
function findCategory(categoryKey) {
  return NAV_CATEGORIES.find((c) => c.key === categoryKey)
    || NAV_CATEGORIES.find((c) => c.key === DEFAULT_CATEGORY);
}

/// First page of a category — where a category click lands.
export function categoryHome(categoryKey) {
  return findCategory(categoryKey).items[0].page;
}

/// antd Menu items for a raw item LIST (e.g. one category object handed out
/// by visibleCategories — permission filtering happens before this point).
export function menuItemsOf(items) {
  return items.map((i) => ({
    key: i.page,
    label: i.menu,
    icon: createElement(i.icon, { 'aria-hidden': true }),
  }));
}

/// antd Menu items for the header brand menu: the first-level tabs only — one
/// flat item per category (key = category key, label = category label, no
/// children), so the popover never repeats the page lists the Sider and the
/// mobile Select already own. Picking a tab navigates to categoryHome(key).
/// The icon is decorative (aria-hidden) so the menuitem's accessible name
/// stays the plain category label.
export function categoryMenuItemsOf(categories) {
  return categories.map((category) => ({
    key: category.key,
    label: category.label,
    icon: createElement(category.icon, { 'aria-hidden': true }),
  }));
}

/// Key of the category owning `page`, or undefined when no handed-in
/// category lists it (unknown page, or a page hidden by permission scoping)
/// — the brand menu then highlights no tab. Pure.
export function categoryKeyOfPage(categories, page) {
  return categories.find((category) => category.items.some((item) => item.page === page))?.key;
}

/// Annotate a flat Menu item list with `aria-current="page"` on the selected
/// item. Pure: returns fresh item objects in a fresh array, never mutates the
/// input (nav.js items are shared by every navigation surface). Items that
/// are not selected get `aria-current: undefined`, which React omits from the
/// DOM instead of rendering an empty attribute.
export function withCurrentPage(items, selected) {
  return items.map((item) => ({ ...item, 'aria-current': item.key === selected ? 'page' : undefined }));
}

/// antd Select options for the mobile picker from a raw item LIST.
export function selectItemsOf(items) {
  return items.map((i) => ({ value: i.page, label: i.menu }));
}

/// Admin sees administration; editor and viewer share all platform pages.
export function visibleCategories(identity) {
  return identity?.role === 'admin' ? NAV_CATEGORIES : NAV_CATEGORIES.filter((category) => category.key !== 'admin');
}

/// Flat page keys a given identity may open (menu scoping + shell routing
/// both ask this; non-admins may open topics and every Ontology page).
export function allowedPages(identity) {
  return visibleCategories(identity).flatMap((c) => c.items.map((i) => i.page));
}
