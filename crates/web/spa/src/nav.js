// nav.js — the single source of truth for the fleet-console information
// architecture (IA): 项目 / Agent / Ontology / 后台管理, each owning
// an ordered page list. The Sider renders scrollable category tabs above a Menu
// scoped to the active category; the active category is PURELY derived from
// the store `page` (no extra global navigation state). `items` is the single
// source for every navigation consumer below.

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
  ApartmentOutlined,
  DatabaseOutlined,
  LinkOutlined,
  SettingOutlined,
  TeamOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';

/// Category → ordered pages. `icon` stores the icon COMPONENT reference (not
/// a JSX element) so this file stays plain .js; `menu` doubles as the mobile
/// Select option label.
export const NAV_CATEGORIES = [
  {
    key: 'project',
    label: '项目',
    items: [
      { page: 'project', menu: '项目', icon: ProjectOutlined },
    ],
  },
  {
    key: 'agent',
    label: 'Agent',
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
    items: [
      { page: 'ontologyGraph', menu: '图谱', icon: ApartmentOutlined },
      { page: 'ontologyEntities', menu: '实体', icon: DatabaseOutlined },
      { page: 'ontologyTypes', menu: '实体类型', icon: ProfileOutlined },
      { page: 'ontologyRelationships', menu: '关系类型', icon: LinkOutlined },
      { page: 'ontologyEnvironments', menu: '环境管理', icon: SettingOutlined },
    ],
  },
  {
    key: 'admin',
    label: '后台管理',
    items: [
      { page: 'nodes', menu: '节点列表', icon: ClusterOutlined },
      { page: 'users', menu: '用户权限', icon: TeamOutlined },
      { page: 'tokens', menu: 'Token 管理', icon: SettingOutlined },
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
export const DEFAULT_PAGE = 'nodes';

/// Category labels for the scrollable tabs (Sider + mobile row 1).
export const CATEGORY_OPTIONS = NAV_CATEGORIES.map((c) => ({ value: c.key, label: c.label }));

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
  brain: 'body-title',
  topics: 'menu-only',
  schedules: 'menu-only',
  dag: 'body-title',
  todos: 'body-title',
  team: 'menu-only',
  chat: 'menu-only',
  agents: 'body-title',
  nodes: 'menu-only',
};

/// Derived from the reasons registry so the two lists can never drift.
export const HEADERLESS_PAGES = Object.keys(HEADERLESS_REASONS);

/// Per-page header copy for the pages whose panel actually mounts PageShell
/// with its own key: pageShell.jsx renders
/// title + desc from here. An entry no panel renders is dead copy — the
/// header contract test mounts every panel and fails on it, so this map
/// stays truthful.
export const PAGE_META = {
  users: { title: '用户权限', desc: '管理用户及其资源权限' },
  tokens: { title: 'Token 管理', desc: '为用户签发和撤销访问 Token' },
  project: { title: '项目', desc: '项目、专项与 TODO 工作台' },
  ontologyGraph: { title: '图谱', desc: '关系观测与切面管理' },
  ontologyEntities: { title: '实体', desc: '目录、属性与正文' },
  ontologyTypes: { title: '实体类型', desc: '类型、属性定义与 Action 配置' },
  ontologyRelationships: { title: '关系类型', desc: '关系约束与关联记录' },
  ontologyEnvironments: { title: '环境管理', desc: 'Ontology 数据环境' },
};

/// Category lookup with the default as the safety net (unknown keys never
/// crash a renderer — they just show the default category).
function findCategory(categoryKey) {
  return NAV_CATEGORIES.find((c) => c.key === categoryKey)
    || NAV_CATEGORIES.find((c) => c.key === DEFAULT_CATEGORY);
}

/// Active category for a store `page`: unknown pages fall back to node.
export function categoryOf(page) {
  const hit = NAV_CATEGORIES.find((c) => c.items.some((i) => i.page === page));
  return hit ? hit.key : DEFAULT_CATEGORY;
}

/// First page of a category — where a category click lands.
export function categoryHome(categoryKey) {
  return findCategory(categoryKey).items[0].page;
}

/// Page keys of one category, in menu order (mobile Select options + tests).
export function pagesOf(categoryKey) {
  return findCategory(categoryKey).items.map((i) => i.page);
}

/// antd Menu items for one category (createElement keeps this file .js).
export function menuOf(categoryKey) {
  return menuItemsOf(findCategory(categoryKey).items);
}

/// antd Select options for the mobile page picker of one category.
export function selectOptionsOf(categoryKey) {
  return selectItemsOf(findCategory(categoryKey).items);
}

/// antd Menu items for a raw item LIST (e.g. one category object handed out
/// by visibleCategories — permission filtering happens before this point).
export function menuItemsOf(items) {
  return items.map((i) => ({
    key: i.page,
    label: i.menu,
    icon: createElement(i.icon),
  }));
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

/// Sider highlight key: anything not in the active category's menu falls
/// back to the default page so the highlight never dangles.
export function menuKey(page) {
  return pagesOf(categoryOf(page)).includes(page) ? page : DEFAULT_PAGE;
}
