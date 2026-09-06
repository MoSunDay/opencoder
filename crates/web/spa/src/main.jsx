// main.jsx — app shell: antd Layout with brand Header + connection badge,
// left Sider (category Segmented on top, then the page Menu scoped to the
// active category), Content switching per page, and the login gate. The
// whole navigation derives from nav.js NAV_CATEGORIES; the active category
// is derived from `page` itself, so there is no second navigation state to
// keep in sync (mobile renders the same two levels as Segmented + Select).
// Visual identity lives in theme.js (antd ThemeConfig) + app.css --oc-*;
// no component here carries an inline color.

import { Alert, Badge, Button, ConfigProvider, Layout, Menu, Segmented, Select, Tooltip, Typography } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import dayjs from 'dayjs';
import { useCallback, useState } from 'react';
import { createRoot } from 'react-dom/client';
import 'dayjs/locale/zh-cn';
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
  CATEGORY_OPTIONS,
  categoryHome,
  categoryOf,
  menuKey,
  menuOf,
  selectOptionsOf,
} from './nav.js';
import { clearCredentials, setState, useStore } from './store.js';
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
  const { token, page } = useStore();
  // Panel→shell notices carry {type, text} (notice.js); normalizeNotice
  // keeps legacy bare-string call sites safe. Empty text (the onNotice('')
  // clear convention) renders nothing.
  const [notice, setNotice] = useState(null);
  // Stable identity: panels key useCallback/useEffect deps on onNotice — a
  // fresh inline arrow per render would re-arm their load effects forever.
  const notify = useCallback((v) => setNotice(normalizeNotice(v)), []);
  // Active category is pure derivation from `page` — clicking a category
  // simply navigates to its home page (nav.js), no extra store field.
  const category = categoryOf(page);

  const goPage = (key) => {
    setState({ page: key });
  };

  return (
    <ConfigProvider theme={theme} locale={zhCN}>
      <div className="fleet-root">
        <Layout className="fleet-layout">
          <Header className="fleet-header">
            <span className="fleet-brand">⛵ Opencoder Fleet</span>
            <div className="fleet-header-side">
              <ConnectionBadge />
              <ServerBase />
              <Button size="small" type="text" onClick={clearCredentials}>退出</Button>
            </div>
          </Header>
          <Layout style={{ minHeight: 0 }}>
            <Sider className="fleet-sidebar" width={200} theme="light">
              <div className="fleet-nav-category">
                <Segmented
                  block
                  value={category}
                  options={CATEGORY_OPTIONS}
                  onChange={(v) => goPage(categoryHome(v))}
                />
              </div>
              <Menu
                mode="inline"
                theme="light"
                selectedKeys={[menuKey(page)]}
                items={menuOf(category)}
                onClick={({ key }) => goPage(key)}
                style={{ borderRight: 0 }}
              />
            </Sider>
            <Content className="fleet-content">
              <Segmented
                className="fleet-mobile-nav"
                block
                value={category}
                options={CATEGORY_OPTIONS}
                onChange={(v) => goPage(categoryHome(v))}
              />
              <Select
                className="fleet-mobile-nav"
                aria-label="页面导航"
                value={menuKey(page)}
                options={selectOptionsOf(category)}
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
                <div className={SHEET_PAGES.has(page) ? 'fleet-sheet' : undefined}>
                  <PageBody page={page} onNotice={notify} />
                </div>
              ) : null}
            </Content>
          </Layout>
        </Layout>
        <LoginModal open={!token} onConnected={() => setNotice(null)} />
      </div>
    </ConfigProvider>
  );
}

export default App;

// Link login bootstraps BEFORE mount (see boot.js — stale-token 401 race).
bootUrlCredential();

// Mount the app — without this the shell serves an empty #root in every
// browser (caught by real-browser acceptance, guarded by an html.rs test).
createRoot(document.getElementById('root')).render(<App />);
