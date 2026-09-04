// @vitest-environment jsdom
// DOM smoke tests for the antd 6 app shell (T2 migration guard):
//   1. render the real <App/> (default export of main.jsx) under jsdom;
//   2. assert the view landmarks users actually see;
//   3. fail the case on ANY deprecation chatter from React/antd on console.
// The pure-node suites (reduce/sign) are frozen — DOM tests live only here.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';

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
import { clearCredentials, setCredentials, setState } from './store.js';

// Unmount the import-time stray app before any test renders its own <App/>.
for (const root of strayRoots.splice(0, strayRoots.length)) {
  root.unmount();
}

// fetch router — request shapes mirror api.js / time.js / chat.jsx: relative
// paths with an empty same-origin base. /api/time → {server_time_ms},
// /api/nodes → {nodes: []}, /api/sessions → {sessions: []}. Nothing here can
// reach a network; unmatched paths resolve to an empty JSON body.
const jsonResponse = (body) => Promise.resolve({
  ok: true,
  status: 200,
  json: () => Promise.resolve(body),
});

const installFetchRouter = () => {
  vi.stubGlobal('fetch', vi.fn((input) => {
    const url = typeof input === 'string' ? input : String((input && input.url) || '');
    if (url.includes('/api/time')) {
      return jsonResponse({ server_time_ms: Date.now() });
    }
    if (url.includes('/api/nodes')) {
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
  expect(hits).toEqual([]);
});

describe('App shell landmarks (antd 6 under jsdom)', () => {
  it('gates unauthenticated visitors behind the login modal', async () => {
    render(<App />);
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    // Nothing renders behind the gate: no fleet table without a token.
    expect(screen.queryByText('暂无 Opencoder 节点')).toBeNull();
  });

  it('shows the empty fleet table on the nodes page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'nodes' });
    render(<App />);
    // findBy*: the table fills in only after the mocked /api/nodes round-trip.
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
  });

  it('shows the empty transcript and the local node on the chat page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'chat' });
    render(<App />);
    expect(await screen.findByText(/选择或新建对话/)).toBeTruthy();
    expect(screen.getByText('本机 (server 本机引擎)')).toBeTruthy();
  });

  it('renders the brand and the menu landmarks', () => {
    setCredentials('smoke-token', '');
    render(<App />);
    expect(screen.getByText(/Opencoder Fleet/)).toBeTruthy();
    expect(screen.getByText('Opencoder 列表')).toBeTruthy();
    expect(screen.getByText('会话交互')).toBeTruthy();
    expect(screen.getByText('DAG 工作流')).toBeTruthy();
  });
});
