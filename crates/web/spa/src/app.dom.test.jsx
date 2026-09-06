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

const installFetchRouter = ({ rejectToken, failNodes } = {}) => {
  vi.stubGlobal('fetch', vi.fn((input, init) => {
    const url = typeof input === 'string' ? input : String((input && input.url) || '');
    if (url.includes('/api/nodes')) {
      const bearer = String((init && init.headers && init.headers.Authorization) || '');
      if (rejectToken && bearer === 'Bearer ' + rejectToken) {
        return jsonResponse({ error: 'unauthorized' }, 401);
      }
      if (failNodes) {
        return jsonResponse({ error: '节点服务不可用' }, 500);
      }
      return jsonResponse({ nodes: [] });
    }
    if (url.includes('/api/sessions')) {
      return jsonResponse({ sessions: [] });
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
    fireEvent.change(screen.getByLabelText('共享密钥 (Token)'), {
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
    // the reopened modal still points at the fleet the link named.
    await waitFor(() => expect(screen.getByLabelText('服务器地址').value).toBe('http://fleet2.example.com'));
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
    // Closable: the close button carries an explicit aria-label, and closing
    // it clears the notice (asserted before the 3s poll can re-arm it).
    fireEvent.click(within(alert).getByRole('button', { name: '关闭' }));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
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

  it('shows the empty transcript and the local node on the chat page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'chat' });
    render(<App />);
    expect(await screen.findByText(/选择或新建对话/)).toBeTruthy();
    expect(screen.getByText('自动调度 / 全部会话')).toBeTruthy();
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
    expect(await screen.findByText('调度与绑定')).toBeTruthy();
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
