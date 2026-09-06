// nav.js — the single source of truth for the fleet-console information
// architecture (IA): three top categories (项目 / Agent / 节点), each owning
// an ordered page list. The Sider renders a category Segmented above a Menu
// scoped to the active category; the active category is PURELY derived from
// the store `page` (no extra global navigation state). Appending a page
// (iteration 4 adds 进展 / Owner 视角 under project) is one new row in
// `items` — every consumer below is a pure function over that data.

import { createElement } from 'react';
import {
  CheckSquareOutlined,
  ClusterOutlined,
  DeploymentUnitOutlined,
  EyeOutlined,
  FundViewOutlined,
  MessageOutlined,
  ProfileOutlined,
  ProjectOutlined,
  RobotOutlined,
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
      { page: 'progress', menu: '进展', icon: FundViewOutlined },
      { page: 'ownerview', menu: 'Owner 视角', icon: EyeOutlined },
    ],
  },
  {
    key: 'agent',
    label: 'Agent',
    items: [
      { page: 'brain', menu: '大脑调度', icon: ThunderboltOutlined },
      { page: 'topics', menu: '全部执行', icon: ProfileOutlined },
      { page: 'dag', menu: 'DAG 工作流', icon: DeploymentUnitOutlined },
      { page: 'todos', menu: 'TODO 管理', icon: CheckSquareOutlined },
      { page: 'team', menu: '团队组队', icon: TeamOutlined },
      { page: 'chat', menu: '会话交互', icon: MessageOutlined },
      { page: 'agents', menu: 'Agent 配置', icon: RobotOutlined },
    ],
  },
  {
    key: 'node',
    label: '节点',
    items: [
      { page: 'nodes', menu: '节点列表', icon: ClusterOutlined },
      { page: 'envs', menu: 'Env 管理', icon: SettingOutlined },
    ],
  },
];

/// Fallbacks mirror the pre-IA shell: unknown pages land on the node
/// category / the nodes page.
export const DEFAULT_CATEGORY = 'node';
export const DEFAULT_PAGE = 'nodes';

/// antd Segmented options for the three categories (Sider + mobile row 1).
export const CATEGORY_OPTIONS = NAV_CATEGORIES.map((c) => ({ value: c.key, label: c.label }));

/// Per-page header copy (title + one-line description). Exported now so the
/// IA has one name per page; the header UI itself arrives in iteration 3.
/// `topic_detail` rides under the 全部执行 semantics of its parent page.
export const PAGE_META = {
  project: { title: '项目', desc: '目标、里程碑与 TODO 的用户策展跟踪' },
  progress: { title: '进展', desc: '里程碑进度、进行中 TODO 与最近项目执行' },
  ownerview: { title: 'Owner 视角', desc: '按目标分组的健康度与待人工介入事项' },
  brain: { title: '大脑调度', desc: '能力绑定与情境化调度入口' },
  topics: { title: '全部执行', desc: '舰队全部执行记录与团队过滤' },
  topic_detail: { title: '执行详情', desc: '单条执行的消息级详情回放' },
  dag: { title: 'DAG 工作流', desc: 'DAG 运行的图视图与步骤工件' },
  todos: { title: 'TODO 管理', desc: '持久化 TODO 工作流的调度与验收' },
  team: { title: '团队组队', desc: '多 agent 团队的组建与执行' },
  chat: { title: '会话交互', desc: '与舰队节点对话的会话工作台' },
  agents: { title: 'Agent 配置', desc: '版本化自定义 agent 的池与引用' },
  nodes: { title: '节点列表', desc: '在线 Opencoder 节点与负载' },
  envs: { title: 'Env 管理', desc: '节点环境变量配置' },
};

/// Category lookup with the default as the safety net (unknown keys never
/// crash a renderer — they just show the default category).
function findCategory(categoryKey) {
  return NAV_CATEGORIES.find((c) => c.key === categoryKey)
    || NAV_CATEGORIES.find((c) => c.key === DEFAULT_CATEGORY);
}

/// Active category for a store `page`: topic_detail folds onto its parent
/// topics page (agent category); unknown pages fall back to node.
export function categoryOf(page) {
  if (page === 'topic_detail') {
    return 'agent';
  }
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
  return findCategory(categoryKey).items.map((i) => ({
    key: i.page,
    label: i.menu,
    icon: createElement(i.icon),
  }));
}

/// antd Select options for the mobile page picker of one category.
export function selectOptionsOf(categoryKey) {
  return findCategory(categoryKey).items.map((i) => ({ value: i.page, label: i.menu }));
}

/// Sider highlight key: parameterized sub-pages fold back onto their parent
/// (topic_detail → topics); anything not in the active category's menu falls
/// back to the default page so the highlight never dangles.
export function menuKey(page) {
  const folded = page === 'topic_detail' ? 'topics' : page;
  return pagesOf(categoryOf(page)).includes(folded) ? folded : DEFAULT_PAGE;
}
