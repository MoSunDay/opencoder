// main.jsx — app shell: antd Layout with brand Header + connection badge,
// left Sider (category Segmented on top, then the page Menu scoped to the
// active category), Content switching per page, and the login gate. The
// whole navigation derives from nav.js NAV_CATEGORIES; the active category
// is derived from `page` itself, so there is no second navigation state to
// keep in sync (mobile renders the same two levels as Segmented + Select).
// Visual identity lives in theme.js (antd ThemeConfig) + app.css --oc-*;
// no component here carries an inline color.
//
// The shell is wrapped in antd <App> so context-held message/notification/
// modal APIs see the theme and the zh-CN locale; panels reach it through
// ui/appMessage.js useMessage() (which falls back to the static API when a
// DOM test mounts one standalone). component={false} keeps it a Fragment:
// no extra .ant-app div between #root and .fleet-root, so the 100vh flex
// chain and every existing landmark query stay byte-identical.

import { Alert, App as AntdApp, Badge, Button, ConfigProvider, Layout, Menu, Segmented, Select, Tooltip, Typography } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import dayjs from 'dayjs';
import { useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import 'dayjs/locale/zh-cn';
import { apiGet } from './api.js';
import { AgentsPanel } from './agentsConfig.jsx';
import { FleetBrainPanel as BrainPanel } from './fleet/brain.jsx';
import { ChatPanel } from './chat.jsx';
import { DagPanel } from './dagPanel.jsx';
import { EnvsPanel } from './envsPanel.jsx';
import { LoginModal } from './login.jsx';
import { FleetNodesPanel as NodesPanel } from './fleet/nodes.jsx';
import { OwnerViewPanel } from './project/ownerViewPanel.jsx';
import { ProgressPanel } from './project/progressPanel.jsx';
import { ProjectPanel } from './project/project.jsx';
import { FleetTeamsPanel as TeamPanel } from './fleet/teams.jsx';
import { TodoPanel } from './todoPanel.jsx';
import { ExecutionsPanel as TopicsPanel } from './fleet/executions.jsx';
import './app.css';
import {
  allowedPages,
  categoryHome,
  menuItemsOf,
  selectItemsOf,
  visibleCategories,
} from './nav.js';
import { clearCredentials, setState, setIdentity, useStore } from './store.js';
import { UsersDrawer } from './admin/usersDrawer.jsx';
import { normalizeNotice } from './notice.js';
import { theme } from './theme.js';
import { bootUrlCredential } from './boot.js';

// zh-CN everywhere: antd built-ins (Modal/Popconfirm buttons) + dayjs
// relative dates (fromNow lands in iteration 3).
dayjs.locale('zh-cn');

const { Header, Sider, Content } = Layout;
const { Text } = Typography;

const CONN_BADGE = {
  ok: { status: 'success', text: '已连接' },
  fail: { status: 'error', text: '连接断开' },
  init: { status: 'default', text: '未连接' },
};

function ConnectionBadge() {
  const { conn } = useStore();
  const b = CONN_BADGE[conn] || CONN_BADGE.init;
  return <Badge status={b.status} text={b.text} />;
}

/// Server base readout for the Header. Empty base = same-origin requests,
/// which is the default embedded deployment; spell that out instead of an
/// awkward blank chip.
function ServerBase() {
  const { base } = useStore();
  const sameOrigin = !base;
  return (
    <Tooltip title={sameOrigin ? '使用同源请求（未配置外部地址）' : 'API 请求指向该地址'}>
      <Text type="secondary" style={{ fontSize: 12 }}>{sameOrigin ? '同源' : base}</Text>
    </Tooltip>
  );
}

/// identity.role → 中文标签（GET /api/me 的词表：admin/root/user）。
const ROLE_LABELS = { admin: '管理员', root: 'Root', user: '普通用户' };

/// Authenticated identity readout for the Header: 「name · 角色」; nothing
/// before the login probe resolves an identity.
function IdentityBadge() {
  const { identity } = useStore();
  if (!identity?.name) {
    return null;
  }
  return <Text type="secondary" style={{ fontSize: 12 }}>{identity.name} · {ROLE_LABELS[identity.role] || identity.role}</Text>;
}

/// Page components keyed by store `page` — one map instead of a ternary
/// chain so adding a page stays one line.
const PANELS = {
  chat: ChatPanel,
  team: TeamPanel,
  topics: TopicsPanel,
  project: ProjectPanel,
  progress: ProgressPanel,
  ownerview: OwnerViewPanel,
  dag: DagPanel,
  todos: TodoPanel,
  envs: EnvsPanel,
  agents: AgentsPanel,
  nodes: NodesPanel,
  brain: BrainPanel,
};

function PageBody({ page, onNotice }) {
  const Panel = PANELS[page] || ChatPanel;
  return <Panel onNotice={onNotice} />;
}

/// Pages whose panels render bare flex surfaces (no Card of their own).
/// They get a white sheet (.fleet-sheet) so they still read as a panel now
/// that Content sits on the gray layout canvas. Card-bearing pages float
/// their own white surfaces directly. chat.jsx is touched lightly in a
/// later iteration; this keeps the shell visually whole meanwhile.
const SHEET_PAGES = new Set(['chat']);

function App() {
  const { token, page, identity } = useStore();
  // Panel→shell notices carry {type, text} (notice.js); normalizeNotice
  // keeps legacy bare-string call sites safe. Empty text (the onNotice('')
  // clear convention) renders nothing.
  const [notice, setNotice] = useState(null);
  // 后台管理抽屉（平台用户），仅 admin 身份可见入口。
  const [usersOpen, setUsersOpen] = useState(false);
  // Stable identity: panels key useCallback/useEffect deps on onNotice — a
  // fresh inline arrow per render would re-arm their load effects forever.
  const notify = useCallback((v) => setNotice(normalizeNotice(v)), []);
  // Permission-scoped IA (nav.js): non-admin identities only ever see the
  // Agent category's 全部执行 page; a stored page outside the allowed set
  // renders the topics panel instead (highlight never dangles).
  const visible = visibleCategories(identity);
  const allowed = allowedPages(identity);
  const shownPage = allowed.includes(page) ? page : 'topics';
  const category = visible.find((c) => c.items.some((i) => i.page === shownPage)) || visible[0];
  const categoryOptions = visible.map((c) => ({ value: c.key, label: c.label }));

  const goPage = (key) => {
    setState({ page: key });
  };

  // Refresh / link-login sessions start with a stored token but no identity
  // (the store never persists /api/me): probe once per token so the badge,
  // admin entries, and role-scoped nav survive a reload without re-login.
  // A 401 clears the credential via api.js (login modal reopens); other
  // failures keep the shell and surface through the connection badge.
  useEffect(() => {
    if (!token) {
      return undefined;
    }
    let live = true;
    apiGet('/api/me')
      .then((me) => {
        if (live) {
          setIdentity(me);
        }
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [token]);

  return (
    <ConfigProvider theme={theme} locale={zhCN}>
      <AntdApp component={false}>
      <div className="fleet-root">
        <Layout className="fleet-layout">
          <Header className="fleet-header">
            <span className="fleet-brand">⛵ Opencoder Fleet</span>
            <div className="fleet-header-side">
              <ConnectionBadge />
              <ServerBase />
              <IdentityBadge />
              {identity?.role === 'admin' ? (
                <Button size="small" onClick={() => setUsersOpen(true)}>后台管理</Button>
              ) : null}
              <Button size="small" type="text" onClick={clearCredentials}>退出</Button>
            </div>
          </Header>
          <Layout style={{ minHeight: 0 }}>
            <Sider className="fleet-sidebar" width={200} theme="light">
              <div className="fleet-nav-category">
                <Segmented
                  block
                  value={category.key}
                  options={categoryOptions}
                  onChange={(v) => goPage(categoryHome(v))}
                />
              </div>
              <Menu
                mode="inline"
                theme="light"
                selectedKeys={[shownPage]}
                items={menuItemsOf(category.items)}
                onClick={({ key }) => goPage(key)}
                style={{ borderRight: 0 }}
              />
            </Sider>
            <Content className="fleet-content">
              <Segmented
                className="fleet-mobile-nav"
                block
                value={category.key}
                options={categoryOptions}
                onChange={(v) => goPage(categoryHome(v))}
              />
              <Select
                className="fleet-mobile-nav"
                aria-label="页面导航"
                value={shownPage}
                options={selectItemsOf(category.items)}
                onChange={goPage}
              />
              {notice && notice.text ? (
                <Alert
                  className="fleet-notice"
                  type={notice.type}
                  showIcon
                  closable={{ 'aria-label': '关闭' }}
                  title={notice.text}
                  onClose={() => setNotice(null)}
                />
              ) : null}
              {token ? (
                <div className={SHEET_PAGES.has(shownPage) ? 'fleet-sheet' : undefined}>
                  <PageBody page={shownPage} onNotice={notify} />
                </div>
              ) : null}
            </Content>
          </Layout>
        </Layout>
        <LoginModal open={!token} onConnected={() => setNotice(null)} />
        <UsersDrawer open={usersOpen} onClose={() => setUsersOpen(false)} onNotice={notify} />
      </div>
      </AntdApp>
    </ConfigProvider>
  );
}

export default App;

// Link login bootstraps BEFORE mount (see boot.js — stale-token 401 race).
bootUrlCredential();

// Mount the app — without this the shell serves an empty #root in every
// browser (caught by real-browser acceptance, guarded by an html.rs test).
createRoot(document.getElementById('root')).render(<App />);
