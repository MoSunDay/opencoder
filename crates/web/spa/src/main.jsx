// main.jsx — app shell: antd Layout with brand Header + connection badge,
// left Sider menu, a Segmented secondary nav pinned to the top of Content,
// Content switching per menu, and the login gate. Sider and Segmented both
// derive from NAV_ITEMS so the two navigations can never drift apart.

import { Badge, Layout, Menu, Segmented, Typography } from 'antd';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { AgentsPanel } from './agentsConfig.jsx';
import { BrainPanel } from './brainPanel.jsx';
import { ChatPanel } from './chat.jsx';
import { DagPanel } from './dagPanel.jsx';
import { EnvsPanel } from './envsPanel.jsx';
import { LoginModal } from './login.jsx';
import { NodesPanel } from './nodes.jsx';
import { ProjectPanel } from './project/project.jsx';
import { TeamPanel } from './teamPanel.jsx';
import { TodoPanel } from './todoPanel.jsx';
import { TopicDetailPanel } from './topicDetail.jsx';
import { TopicsPanel } from './topicsPanel.jsx';
import './app.css';
import { setState, useStore } from './store.js';

const { Header, Sider, Content } = Layout;
const { Text } = Typography;

/// One row per page: `menu` is the Sider label, `nav` the Segmented label.
const NAV_ITEMS = [
  { key: 'nodes', menu: 'Opencoder 列表', nav: '节点' },
  { key: 'chat', menu: '会话交互', nav: '会话' },
  { key: 'project', menu: '项目', nav: '项目' },
  { key: 'dag', menu: 'DAG 工作流', nav: 'DAG' },
  { key: 'todos', menu: 'TODO 管理', nav: 'TODO' },
  { key: 'envs', menu: 'Env 管理', nav: 'Env' },
  { key: 'agents', menu: 'Agent 配置', nav: 'Agents' },
  { key: 'team', menu: '团队组队', nav: '组队' },
  { key: 'topics', menu: '团队话题', nav: '话题' },
  { key: 'brain', menu: '项目目标', nav: '目标' },
];
const MENU_ITEMS = NAV_ITEMS.map((i) => ({ key: i.key, label: i.menu }));
const NAV_OPTIONS = NAV_ITEMS.map((i) => ({ value: i.key, label: i.nav }));

/// Sider highlight: parameterized sub-pages fold back onto their parent.
function menuKey(page) {
  return page === 'topic_detail' ? 'topics' : (NAV_ITEMS.some((i) => i.key === page) ? page : 'nodes');
}

/// Segmented value: undefined on sub-pages so clicking the parent option
/// still fires onChange and navigates up (same-value clicks never fire).
function navValue(page) {
  return NAV_ITEMS.some((i) => i.key === page) ? page : undefined;
}

const CONN_BADGE = {
  ok: { status: 'success', text: '已连接' },
  fail: { status: 'error', text: '连接断开' },
  init: { status: 'default', text: '未连接' },
};

function ConnectionBadge() {
  const { conn } = useStore();
  const b = CONN_BADGE[conn] || CONN_BADGE.init;
  return <Badge status={b.status} text={<Text style={{ color: 'rgba(255,255,255,0.85)' }}>{b.text}</Text>} />;
}

/// Page components keyed by store `page` — one map instead of a ternary
/// chain so adding a tab is a single row (sub-pages fold onto their parent).
const PANELS = {
  brain: BrainPanel,
  chat: ChatPanel,
  project: ProjectPanel,
  dag: DagPanel,
  todos: TodoPanel,
  envs: EnvsPanel,
  agents: AgentsPanel,
  team: TeamPanel,
  topics: TopicsPanel,
  topic_detail: TopicDetailPanel,
  nodes: NodesPanel,
};

function PageBody({ page, onNotice }) {
  const Panel = PANELS[page] || ChatPanel;
  return <Panel onNotice={onNotice} />;
}

function App() {
  const { token, page } = useStore();
  const [notice, setNotice] = useState('');

  // Direct nav lands on a fresh view: the topics tab drops the team filter
  // and any topic-detail params (组队's 查看话题 re-arms the filter via
  // openTopicsForTeam).
  const goPage = (key) => {
    if (key === 'topics') {
      setState({ page: 'topics', topicsTeamFilter: null, topicDetail: null });
      return;
    }
    setState({ page: key });
  };

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <Layout style={{ flex: 1, minHeight: 0 }}>
        <Header style={{ display: 'flex', alignItems: 'center', gap: 24, background: '#001529', paddingLeft: 24 }}>
          <span style={{ color: '#fff', fontSize: 17, fontWeight: 700, letterSpacing: 1 }}>
            ⛵ Opencoder Fleet
          </span>
          <ConnectionBadge />
        </Header>
        <Layout style={{ minHeight: 0 }}>
          <Sider width={200} theme="dark">
            <Menu
              mode="inline"
              theme="dark"
              selectedKeys={[menuKey(page)]}
              items={MENU_ITEMS}
              onClick={({ key }) => goPage(key)}
              style={{ height: '100%', borderRight: 0 }}
            />
          </Sider>
          <Content style={{ padding: 20, overflow: 'auto', background: '#fff' }}>
            <Segmented
              value={navValue(page)}
              options={NAV_OPTIONS}
              onChange={(v) => goPage(v)}
              style={{ marginBottom: 16 }}
            />
            {notice ? (
              <div style={{ marginBottom: 12 }}>
                <Typography.Text type="danger">{notice}</Typography.Text>
              </div>
            ) : null}
            {token ? <PageBody page={page} onNotice={setNotice} /> : null}
          </Content>
        </Layout>
      </Layout>
      <LoginModal open={!token} />
    </div>
  );
}

export default App;

// Mount the app — without this the shell serves an empty #root in every
// browser (caught by real-browser acceptance, guarded by an html.rs test).
createRoot(document.getElementById('root')).render(<App />);
