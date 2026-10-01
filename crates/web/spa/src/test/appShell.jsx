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
import '../test/setup-dom.js';
import App from '../main.jsx';
import { LoginModal } from '../login.jsx';
import { bootUrlCredential } from '../boot.js';
import { clearCredentials, embeddedBase, getState, setCredentials, setState } from '../store.js';
// Nav registry: HEADERLESS_REASONS['menu-only'] promises the sidebar Menu AND
// the mobile Select label are that page's only name, so the mobile half of the
// promise is asserted here (the panel half lives in shell/headerContract).
import { HEADERLESS_REASONS, NAV_CATEGORIES } from '../nav.js';
// Locale/theme probe components — same wiring main.jsx uses for the shell,
// imported here so the zh-CN assertion exercises the exact same objects.
import { ConfigProvider, Popconfirm } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import dayjs from 'dayjs';
import { theme } from '../theme.js';

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


export { installFetchRouter, jsonResponse, deprecationHits };
export { default as App } from '../main.jsx';
export { LoginModal } from '../login.jsx';
export { bootUrlCredential } from '../boot.js';
export { clearCredentials, embeddedBase, getState, setCredentials, setState } from '../store.js';
export { HEADERLESS_REASONS, NAV_CATEGORIES } from '../nav.js';
export { ConfigProvider, Popconfirm } from 'antd';
export { default as zhCN } from 'antd/locale/zh_CN';
export { default as dayjs } from 'dayjs';
export { theme } from '../theme.js';
export async function mountApp() {
  const view = render(<App />);
  await waitFor(() => expect(!getState().token || !!getState().identity).toBe(true));
  return view;
}
