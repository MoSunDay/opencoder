// @vitest-environment jsdom
// DOM smoke tests for the antd 6 app shell (T2 migration guard):
//   1. render the real <App/> (default export of main.jsx) under jsdom;
//   2. assert the view landmarks users actually see;
//   3. fail the case on ANY deprecation chatter from React/antd on console.
// The pure-node suites (reduce/sign) are frozen — DOM tests live only here.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

// main.jsx calls createRoot(document.getElementById('root')).render(<App/>)
// at import time. That stray instance would double every landmark query (its
// login Modal portals straight into document.body), so the root is captured
// here and unmounted right after the imports. React Testing Library itself
// uses createRoot from the same module — the wrapper is pass-through, so RTL
// keeps working; its roots are created later, inside the tests, and never end
// up in the list below.
const { strayRoots } = vi.hoisted(() => ({ strayRoots: [] }));
vi.mock('react-dom/client', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    createRoot: (...args) => {
      const root = actual.createRoot(...args);
      strayRoots.push(root);
      return root;
    },
  };
});

// setup-dom.js must run BEFORE main.jsx: it installs the browser shims and the
// #root fixture that main.jsx's import-time mount requires.
import './test/setup-dom.js';
import App from './main.jsx';
import { LoginModal } from './login.jsx';
import { bootUrlCredential } from './boot.js';
import { clearCredentials, embeddedBase, getState, setCredentials, setState } from './store.js';
// Locale/theme probe components — same wiring main.jsx uses for the shell,
// imported here so the zh-CN assertion exercises the exact same objects.
import { ConfigProvider, Popconfirm } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import dayjs from 'dayjs';
import { theme } from './theme.js';

// Unmount the import-time stray app before any test renders its own <App/>.
for (const root of strayRoots.splice(0, strayRoots.length)) {
  root.unmount();
}

// fetch router — request shapes mirror api.js / chat.jsx: relative paths with
// an empty same-origin base. /api/nodes → {nodes: []}, /api/sessions →
// {sessions: []}. Nothing here can
// reach a network; unmatched paths resolve to an empty JSON body.
const jsonResponse = (body, status = 200) => Promise.resolve({
  ok: status >= 200 && status < 300,
  status,
  json: () => Promise.resolve(body),
});

const installFetchRouter = ({ rejectToken, failNodes, withGoal } = {}) => {
  vi.stubGlobal('fetch', vi.fn((input, init) => {
    const url = typeof input === 'string' ? input : String((input && input.url) || '');
    const bearer = String((init && init.headers && init.headers.Authorization) || '');
    if (rejectToken && bearer === 'Bearer ' + rejectToken) {
      // Any protected surface (nodes fetch, /api/me identity probe) rejects.
      return jsonResponse({ error: 'unauthorized' }, 401);
    }
    if (url.includes('/api/nodes')) {
      if (failNodes) {
        return jsonResponse({ error: '节点服务不可用' }, 500);
      }
      return jsonResponse({ nodes: [] });
    }
    if (url.includes('/api/sessions')) {
      return jsonResponse({ sessions: [] });
    }
    if (url.includes('/api/me')) {
      return jsonResponse({ name: 'smoke', role: 'admin' });
    }
    if (withGoal && url.includes('/api/project/overview')) {
      // Seed one active goal: GoalsTab's EMPTY branch carries no MdEditModal,
      // so the create-goal modal only exists once a goal is in the list.
      return jsonResponse({
        goals: [{ id: 'g-seed', title: '既有目标', status: 'active', sort: 0, milestones: [] }],
        backlog: [],
      });
    }
    return jsonResponse({});
  }));
};

// Console capture: the antd 5→6 migration is only complete when rendering is
// silent — antd/React announce removed APIs via console.error/warn carrying
// the word "deprecated" (e.g. `destroyOnClose`, `maskClosable`).
const consoleLog = { error: [], warn: [] };
const record = (bucket) => (...args) => {
  consoleLog[bucket].push(args.map((a) => String(a)).join(' '));
};
const deprecationHits = () => consoleLog.error.concat(consoleLog.warn)
  .filter((line) => /deprecated/i.test(line));

// The store is a module-level singleton on useSyncExternalStore: every test
// starts from the same clean slate (fresh localStorage, no credentials, fleet
// tab) so cases never leak state into each other.
beforeEach(() => {
  window.history.replaceState(null, '', '/');
  localStorage.clear();
  clearCredentials();
  setState({ page: 'nodes', preselectNode: null, nodes: [], conn: 'init' });
  installFetchRouter();
  vi.spyOn(console, 'error').mockImplementation(record('error'));
  vi.spyOn(console, 'warn').mockImplementation(record('warn'));
});

afterEach(() => {
  cleanup();
  const hits = deprecationHits();
  consoleLog.error = [];
  consoleLog.warn = [];
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  expect(hits).toEqual([]);
});

describe('App shell landmarks (antd 6 under jsdom)', () => {
  it('gates unauthenticated visitors behind the login modal', async () => {
    render(<App />);
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    // Nothing renders behind the gate: no fleet table without a token.
    expect(screen.queryByText('暂无 Opencoder 节点')).toBeNull();
  });

  it('notifies the app to clear stale request errors after a successful login', async () => {
    const onConnected = vi.fn();
    render(<LoginModal open onConnected={onConnected} />);
    fireEvent.change(screen.getByLabelText('访问令牌 (Token)'), {
      target: { value: 'correct-token' },
    });
    fireEvent.click(screen.getByRole('button', { name: /连\s*接/ }));
    await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
    expect(localStorage.getItem('oc_token')).toBe('correct-token');
  });

  it('logs in from a ?token= link, scrubbing the secret from the URL', async () => {
    window.history.replaceState(null, '', '/?view=nodes&token=url-secret#fleet');
    bootUrlCredential();
    render(<App />);
    // No login modal: the fleet table renders straight away.
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(window.location.search).toBe('?view=nodes');
    expect(window.location.hash).toBe('#fleet');
    expect(localStorage.getItem('oc_token')).toBe('url-secret');
  });

  it('logs in from a #token= fragment link and scrubs it', async () => {
    window.history.replaceState(null, '', '/#token=frag-secret');
    bootUrlCredential();
    render(<App />);
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(window.location.hash).toBe('');
    expect(localStorage.getItem('oc_token')).toBe('frag-secret');
  });

  it('a URL token overrides stored credentials for authenticated visitors', async () => {
    // The race this case pins down: the stale token must never win — boot
    // adopts the URL token before any component can fire a 401-ing request.
    setCredentials('stored-token', '');
    window.history.replaceState(null, '', '/?token=url-secret#fleet');
    bootUrlCredential();
    render(<App />);
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(window.location.search).toBe('');
    expect(localStorage.getItem('oc_token')).toBe('url-secret');
  });

  it('falls back to the login modal when a URL token is rejected (401)', async () => {
    installFetchRouter({ rejectToken: 'bad-secret' });
    window.history.replaceState(null, '', '/?token=bad-secret');
    bootUrlCredential();
    render(<App />);
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    expect(window.location.search).toBe('');
    expect(localStorage.getItem('oc_token')).toBeNull();
  });

  it('keeps the URL-delivered base when the URL token is rejected (401)', async () => {
    installFetchRouter({ rejectToken: 'bad-secret' });
    window.history.replaceState(null, '', '/#token=bad-secret&base=http://fleet2.example.com');
    bootUrlCredential();
    render(<App />);
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    // The 401 cleared the token but must not wipe the link-delivered base:
    // login is token-only now (no 服务器地址 field), so the surviving base is
    // the store/localStorage one the next probe reuses.
    expect(screen.queryByLabelText('服务器地址')).toBeNull();
    expect(getState().base).toBe('http://fleet2.example.com');
    expect(localStorage.getItem('oc_base')).toBe('http://fleet2.example.com');
  });

  it('adopts a base-only link: base stored, session token kept, url scrubbed', () => {
    setCredentials('stored-token', '');
    window.history.replaceState(null, '', '/#base=http://fleet2.example.com');
    bootUrlCredential();
    expect(window.location.hash).toBe('');
    expect(localStorage.getItem('oc_token')).toBe('stored-token');
    expect(localStorage.getItem('oc_base')).toBe('http://fleet2.example.com');
    expect(getState().base).toBe('http://fleet2.example.com');
  });

  it('embeddedBase() honors a baked VITE_OC_BASE (trimmed, slash-stripped)', () => {
    vi.stubEnv('VITE_OC_BASE', '  https://fleet.example.com/  ');
    expect(embeddedBase()).toBe('https://fleet.example.com');
    vi.stubEnv('VITE_OC_BASE', '');
    expect(embeddedBase()).toBe('');
  });

  it('re-probes /api/me on refresh and restores the admin identity', async () => {
    // A page reload (or a ?token= link login) restores the token but not the
    // identity — the store never persists /api/me. The shell must probe on
    // mount so the badge, admin entry, and admin nav come back by themselves.
    setCredentials('smoke-token', '');
    render(<App />);
    expect(await screen.findByText('smoke · 管理员')).toBeTruthy();
    expect(await screen.findByRole('button', { name: '后台管理' })).toBeTruthy();
    expect(getState().identity).toEqual({ name: 'smoke', role: 'admin' });
    // No login modal: the stored credential was accepted.
    expect(screen.queryByText('Opencoder Fleet · 登录')).toBeNull();
  });

  it('drops a stored token the refresh probe rejects (401) back to the login modal', async () => {
    installFetchRouter({ rejectToken: 'stale-token' });
    setCredentials('stale-token', '');
    render(<App />);
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    expect(localStorage.getItem('oc_token')).toBeNull();
    expect(getState().identity).toBeNull();
  });

  it('shows the empty fleet table on the nodes page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'nodes' });
    render(<App />);
    // findBy*: the table fills in only after the mocked /api/nodes round-trip.
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
  });

  it('surfaces panel errors as a closable error Alert notice', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'nodes' });
    installFetchRouter({ failNodes: true });
    render(<App />);
    // The nodes panel routes its load failure through onNotice, which the
    // shell now renders as an antd Alert (role=alert) instead of red text.
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('节点服务不可用');
    // R1: the shell renders the notice's own severity — a load failure stays
    // a red error Alert (ant-alert-error), not a generic one.
    expect(alert.className).toContain('ant-alert-error');
    // Closable: the close button carries an explicit aria-label, and closing
    // it clears the notice (asserted before the 3s poll can re-arm it).
    fireEvent.click(within(alert).getByRole('button', { name: '关闭' }));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('renders a success Alert after a real goal-create round-trip on the project page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'project' });
    installFetchRouter({ withGoal: true });
    render(<App />);
    // Real interaction flow: open the goals tab, 新建目标 modal, fill the
    // form, submit — the fetch router answers 200 so GoalsTab reports
    // ok('目标已创建') and the shell paints it green (R1 fix).
    fireEvent.click(await screen.findByRole('tab', { name: '项目目标' }));
    await act(async () => { await new Promise((r) => setTimeout(r, 20)); });
    fireEvent.click(await screen.findByText('新建目标'));
    await act(async () => { await new Promise((r) => setTimeout(r, 20)); });
    fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '新目标' } });
    // antd auto-inserts a space between the CJK glyphs (保 存) — strip it.
    const save = [...document.querySelectorAll('button')].find((b) => b.textContent.replace(/\s+/g, '') === '保存');
    expect(save).toBeTruthy();
    fireEvent.click(save);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('目标已创建');
    expect(alert.className).toContain('ant-alert-success');
    expect(alert.className).not.toContain('ant-alert-error');
  });

  it('logs out from the Header: token cleared, login gate reopens', async () => {
    setCredentials('smoke-token', '');
    render(<App />);
    // The header's right side carries the same-origin readout + 退出.
    expect(screen.getByText('同源')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '退出' }));
    // clearCredentials(): persisted token gone, state token '' → gate reopens.
    expect(localStorage.getItem('oc_token')).toBeNull();
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
  });

  it('renders antd built-ins in zh-CN under the shell ConfigProvider', async () => {
    // main.jsx wraps the whole shell (login Modal included) in
    // <ConfigProvider theme={theme} locale={zhCN}>. Its default zh-CN button
    // texts only surface inside built-in popups (Popconfirm/Modal), none of
    // which are reachable in a gated shell — so this probe mounts one under
    // the exact same theme + locale imports and pins the default texts.
    render(
      <ConfigProvider theme={theme} locale={zhCN}>
        <Popconfirm title="确认删除？" open>
          <span>probe</span>
        </Popconfirm>
      </ConfigProvider>
    );
    // antd inserts a space between the two CJK glyphs of each default
    // button (autoInsertSpaceInButton), so match with \s* like 连 接 above.
    expect(await screen.findByRole('button', { name: /确\s*定/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /取\s*消/ })).toBeTruthy();
    // dayjs locale is a main.jsx module side effect (relative dates arrive
    // in a later iteration); importing the shell must have switched it.
    expect(dayjs.locale()).toBe('zh-cn');
  });

  it('requires an explicit execution node on the chat page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'chat' });
    render(<App />);
    expect(await screen.findByText(/选择或新建对话/)).toBeTruthy();
    expect(screen.getByText('请先选择执行节点')).toBeTruthy();
    expect(screen.getByPlaceholderText('输入提示词，Enter 发送，Shift+Enter 换行').disabled).toBe(true);
  });

  it('renders the brand and the node-category menu on the default page', () => {
    setCredentials('smoke-token', '');
    render(<App />);
    expect(screen.getByText(/Opencoder Fleet/)).toBeTruthy();
    // Default page (nodes) scopes the Sider menu to the node category. Icon
    // glyphs carry their own aria-label, so match menuitem names by regex.
    expect(screen.getByRole('menuitem', { name: /节点列表/ })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: /Env 管理/ })).toBeTruthy();
    // Pages of other categories stay out of the scoped menu.
    expect(screen.queryByRole('menuitem', { name: /会话交互/ })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: /DAG 工作流/ })).toBeNull();
  });

  it('drops the retired desktop Segmented nav from the Content top', () => {
    setCredentials('smoke-token', '');
    render(<App />);
    // The old always-on page Segmented (fleet-desktop-nav) is gone for good.
    expect(document.querySelector('.fleet-desktop-nav')).toBeNull();
    // Whatever Segmented remains inside Content is the mobile-only category
    // switch, tagged with the fleet-mobile-nav show/hide class.
    const segments = Array.from(document.querySelectorAll('.fleet-content .ant-segmented'));
    expect(segments.length).toBeGreaterThan(0);
    for (const el of segments) {
      expect(el.classList.contains('fleet-mobile-nav')).toBe(true);
    }
  });

  it('switches categories via the Sider Segmented and lands on the home page', async () => {
    setCredentials('smoke-token', '');
    render(<App />);
    // Scope to the Sider Segmented — its mobile twin also sits in the DOM
    // (hidden only by the media query), so unscoped text queries would hit
    // both.
    const sider = within(document.querySelector('.fleet-sidebar'));
    fireEvent.click(sider.getByText('Agent'));
    // A category click navigates to its first page (brain)…
    expect(getState().page).toBe('brain');
    // …re-scopes + highlights the menu item…
    const brainItem = screen.getByRole('menuitem', { name: /大脑调度/ });
    expect(brainItem.classList.contains('ant-menu-item-selected')).toBe(true);
    // …and the panel behind it renders (agent-category page).
    expect(await screen.findByText('需求执行')).toBeTruthy();
  });

  it('scopes the project category to its three menu items', async () => {
    setCredentials('smoke-token', '');
    render(<App />);
    const sider = within(document.querySelector('.fleet-sidebar'));
    fireEvent.click(sider.getByText('项目'));
    expect(getState().page).toBe('project');
    // Menuitem names carry the icon glyph's own aria-label → match by regex
    // (same convention as the surrounding landmark assertions).
    expect(await screen.findByRole('menuitem', { name: /项目/ })).toBeTruthy();
    // Iteration 4 pages ride under the same category menu.
    expect(screen.getByRole('menuitem', { name: /进展/ })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: /Owner 视角/ })).toBeTruthy();
    expect(screen.queryByRole('menuitem', { name: /大脑调度/ })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: /节点列表/ })).toBeNull();
  });
});
