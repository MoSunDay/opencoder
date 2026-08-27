// main.jsx — app shell: antd Layout with brand Header + connection badge,
// left Sider menu (exactly two items), Content switching per menu, the
// plaintext-HTTP warning banner, and the login gate.

import { Badge, Layout, Menu, Typography } from 'antd';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ChatPanel } from './chat.jsx';
import { InsecureHttpAlert, LoginModal } from './login.jsx';
import { NodesPanel } from './nodes.jsx';
import './app.css';
import { setState, useStore } from './store.js';

const { Header, Sider, Content } = Layout;
const { Text } = Typography;

const MENU_ITEMS = [
  { key: 'nodes', label: 'Opencoder 列表' },
  { key: 'chat', label: '会话交互' },
];

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

function App() {
  const { token, page } = useStore();
  const [notice, setNotice] = useState('');

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <InsecureHttpAlert />
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
              selectedKeys={[page]}
              items={MENU_ITEMS}
              onClick={({ key }) => setState({ page: key })}
              style={{ height: '100%', borderRight: 0 }}
            />
          </Sider>
          <Content style={{ padding: 20, overflow: 'auto', background: '#fff' }}>
            {notice ? (
              <div style={{ marginBottom: 12 }}>
                <Typography.Text type="danger">{notice}</Typography.Text>
              </div>
            ) : null}
            {token
              ? (page === 'nodes'
                ? <NodesPanel onNotice={setNotice} />
                : <ChatPanel onNotice={setNotice} />)
              : null}
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
