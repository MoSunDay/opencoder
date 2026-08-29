// @vitest-environment jsdom
// T5 sidebar DOM smoke: the chat page's @ant-design/x Conversations list.
// Same contract style as chat.dom.test.jsx — fetch is stubbed with a
// URL-routed mock (fixtures swappable per case), everything above the
// protocol layer runs for real. Landmarks verified in @ant-design/x 2.9
// sources (es/conversations/):
//   list root   <ul class="ant-conversations">
//   item        <li class="ant-conversations-item">  (+ -active highlight)
//   item label  .ant-conversations-label
//   creation    <button class="ant-conversations-creation">
// The two dialog sources stay wired to the real loaders:
//   local  GET /api/sessions?limit=50 → {sessions}
//   remote GET /api/nodes/:id/dialogs → {dialogs}

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// main.jsx mounts <App/> at import time — capture and unmount that stray root
// exactly like app.dom.test.jsx so landmark queries are not doubled.
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

import './test/setup-dom.js';
import App from './main.jsx';
import { ChatPanel } from './chat.jsx';
import { clearCredentials, getState, openChatForNode, setCredentials, setNodes, setState } from './store.js';

for (const root of strayRoots.splice(0, strayRoots.length)) {
  root.unmount();
}

// Per-case fixtures, swapped by the tests before (re)installing the router.
const fixtures = {
  localSessions: [],
  nodeDialogs: { dialogs: [] },
  snapshot: { messages: [] },
};

let hits = [];

const jsonResponse = (body) => Promise.resolve({
  ok: true,
  status: 200,
  json: () => Promise.resolve(body),
});

/// A 200 whose body stream never closes — the live-SSE stand-in (sse.js's
/// readLoop stays pending, so no reconnect noise while the case finishes).
const hangingStreamResponse = () => new Response(
  new ReadableStream({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(''));
    },
  }),
  { status: 200, headers: { 'content-type': 'text/event-stream' } },
);

const installRouter = () => {
  hits = [];
  vi.stubGlobal('fetch', vi.fn((input, opts = {}) => {
    const url = typeof input === 'string' ? input : String((input && input.url) || '');
    const method = String(opts.method || 'GET').toUpperCase();
    hits.push({ method, url, body: opts.body || '' });
    if (url.includes('/api/time')) {
      return jsonResponse({ server_time_ms: Date.now() });
    }
    // Node dialog index — must match before the generic /api/nodes route.
    if (url.includes('/dialogs')) {
      return jsonResponse(fixtures.nodeDialogs);
    }
    if (url.includes('/events')) {
      return hangingStreamResponse();
    }
    if (url.includes('/seq')) {
      return jsonResponse({ seq: 0 });
    }
    if (url.includes('/prompt') || url.includes('/interrupt')) {
      return jsonResponse({ ok: true });
    }
    if (url === '/api/sessions' || url.startsWith('/api/sessions?')) {
      return method === 'POST' ? jsonResponse({ id: 'new-1' }) : jsonResponse({ sessions: fixtures.localSessions });
    }
    if (url.includes('/api/nodes')) {
      return jsonResponse({ nodes: [] });
    }
    // GET /api/sessions/:id — the transcript snapshot openDialog loads.
    return jsonResponse(fixtures.snapshot);
  }));
};

const consoleLog = { error: [], warn: [] };
const record = (bucket) => (...args) => {
  consoleLog[bucket].push(args.map((a) => String(a)).join(' '));
};
const deprecationHits = () => consoleLog.error.concat(consoleLog.warn)
  .filter((line) => /deprecated/i.test(line));

beforeEach(() => {
  localStorage.clear();
  clearCredentials();
  // Store is a module-level singleton — reset it so cases never leak state.
  setState({ page: 'nodes', preselectNode: null, nodes: [], conn: 'init' });
  fixtures.localSessions = [];
  fixtures.nodeDialogs = { dialogs: [] };
  fixtures.snapshot = { messages: [] };
  installRouter();
  vi.spyOn(console, 'error').mockImplementation(record('error'));
  vi.spyOn(console, 'warn').mockImplementation(record('warn'));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  expect(deprecationHits()).toEqual([]);
});

const localSessionsFixture = () => [
  { id: 's1', title: '修复登录页', created_at: 1000, updated_at: 2000 },
  { id: 's2', title: null, created_at: 1000, updated_at: 2000 },
];

const snapshotFixture = () => ({
  messages: [
    { role: 'user', blocks: [{ kind: 'text', text: '帮我看看登录页' }] },
    { role: 'assistant', blocks: [{ kind: 'text', text: '好的，开始排查' }] },
  ],
});

const mountChat = async () => {
  setCredentials('smoke-token', '');
  const renderResult = render(<ChatPanel />);
  await waitFor(() => {
    expect(hits.some((h) => h.url.startsWith('/api/sessions?'))).toBe(true);
  });
  return renderResult;
};

/// Open the antd 6 Select dropdown and click the option labelled `label`.
/// antd 6 dropped the old `.ant-select-selector` wrapper — the interactive
/// surface is the `.ant-select` root, options portal into document.body as
/// `.ant-select-item-option` with a `title` attribute (probed under jsdom).
const pickSelectOption = async (container, label) => {
  await act(async () => {
    fireEvent.mouseDown(container.querySelector('.ant-select'));
  });
  const option = await waitFor(() => {
    const all = [...document.querySelectorAll('.ant-select-item-option')];
    const hit = all.find((o) => o.getAttribute('title') === label || o.textContent === label);
    expect(hit).toBeTruthy();
    return hit;
  });
  await act(async () => {
    fireEvent.click(option);
  });
};

const conversationsItem = (label) => [...document.querySelectorAll('li.ant-conversations-item')]
  .find((li) => (li.querySelector('.ant-conversations-label') || {}).textContent === label);

describe('Conversations sidebar (T5 two-column chat)', () => {
  it('renders the local sessions as items, then the remote list after a node switch', async () => {
    fixtures.localSessions = localSessionsFixture();
    fixtures.nodeDialogs = {
      dialogs: [{ session_id: 'r1', title: '远端会话', first_created_at: 1, last_created_at: 2, task_count: 3 }],
    };
    // The fleet option in the node select comes from the shared store.
    setState({ nodes: [{ id: 'n1', name: 'Fleet-1' }] });
    const { container } = await mountChat();

    // Local source: both sessions rendered with their mapped labels.
    expect(conversationsItem('修复登录页')).toBeTruthy();
    expect(conversationsItem('s2…')).toBeTruthy();
    expect(document.querySelectorAll('li.ant-conversations-item')).toHaveLength(2);

    // Switch to the fleet node → the loader pulls /api/nodes/n1/dialogs.
    await pickSelectOption(container, 'Fleet-1');
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/api/nodes/n1/dialogs'))).toBe(true);
    });
    await waitFor(() => {
      expect(document.querySelectorAll('li.ant-conversations-item')).toHaveLength(1);
    });
    expect(conversationsItem('远端会话')).toBeTruthy();
    expect(conversationsItem('修复登录页')).toBeUndefined();
  });

  it('loads the clicked dialog through the openDialog path', async () => {
    fixtures.localSessions = localSessionsFixture();
    fixtures.snapshot = snapshotFixture();
    await mountChat();

    const item = conversationsItem('修复登录页');
    await act(async () => {
      fireEvent.click(item);
    });
    // openDialog(session_id) → GET /api/sessions/:id snapshot fetch.
    await waitFor(() => {
      expect(hits.some((h) => h.method === 'GET' && h.url === '/api/sessions/s1')).toBe(true);
    });
    // The snapshot turns render in the transcript pane.
    await waitFor(() => {
      expect(screen.getByText('帮我看看登录页')).toBeTruthy();
    });
  });

  it('highlights the active item with the -active class once selected', async () => {
    fixtures.localSessions = localSessionsFixture();
    fixtures.snapshot = snapshotFixture();
    const { container } = await mountChat();

    expect(container.querySelector('li.ant-conversations-item-active')).toBeNull();
    await act(async () => {
      fireEvent.click(conversationsItem('s2…'));
    });
    await waitFor(() => {
      const active = container.querySelector('li.ant-conversations-item-active .ant-conversations-label');
      expect(active).toBeTruthy();
      expect(active.textContent).toBe('s2…');
    });
  });

  it('starts a new chat from the creation button and clears the active item', async () => {
    fixtures.localSessions = localSessionsFixture();
    fixtures.snapshot = snapshotFixture();
    const { container } = await mountChat();

    await act(async () => {
      fireEvent.click(conversationsItem('修复登录页'));
    });
    await waitFor(() => {
      expect(container.querySelector('li.ant-conversations-item-active')).toBeTruthy();
    });

    const create = container.querySelector('button.ant-conversations-creation');
    expect(create).toBeTruthy();
    expect(create.textContent).toContain('新建对话');
    await act(async () => {
      fireEvent.click(create);
    });
    // Same reset pair the old header button ran: empty state back, no active.
    await waitFor(() => {
      expect(container.querySelector('li.ant-conversations-item-active')).toBeNull();
    });
    expect(screen.getByText('选择或新建对话，输入提示词开始')).toBeTruthy();
  });

  it('lands the fleet tab preselect on the sidebar node select', async () => {
    setNodes([{ id: 'n1', name: 'Fleet-1' }]);
    openChatForNode('n1');
    setCredentials('smoke-token', '');
    render(<App />);

    // Chat page is the active tab and the node select shows the preselect.
    expect(await screen.findByText('Fleet-1')).toBeTruthy();
    expect(getState().page).toBe('chat');
    // The preselect effect consumed the request and loaded that node's list.
    expect(getState().preselectNode).toBeNull();
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/api/nodes/n1/dialogs'))).toBe(true);
    });
  });
});
