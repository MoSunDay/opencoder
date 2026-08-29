// @vitest-environment jsdom
// Chat tab smoke tests for the @ant-design/x migration (T3 Bubble.List +
// T4 Sender). The protocol layers (sse.js / api.js / sign.js) are consumed
// read-only: fetch is stubbed with a URL-routed mock, everything above it —
// reduce, chat.jsx, transcript.jsx — runs for real. DOM landmarks use the
// class prefixes shipped by @ant-design/x 2.9 (verified in node_modules):
//   Bubble.List root  → .ant-bubble-list / .ant-bubble[-start|-end]
//   Sender textarea   → textarea.ant-sender-input
//   Sender stop btn   → .ant-sender-actions-btn-loading-button
// Sender submits on a keydown of key='Enter' without shift/ctrl/alt/meta and
// outside IME composition (sender/components/TextArea.js onInternalKeyDown).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// setup-dom.js installs the browser shims (matchMedia, observers) and the
// RTL afterEach(cleanup) — import it before any component module.
import './test/setup-dom.js';
import { ChatPanel } from './chat.jsx';
import { TranscriptView } from './transcript.jsx';
import { clearCredentials, setCredentials, setState } from './store.js';

// Every request is recorded so assertions can inspect method + signed body.
let hits = [];

const jsonResponse = (body) => Promise.resolve({
  ok: true,
  status: 200,
  json: () => Promise.resolve(body),
});

/// A 200 Response whose body stream never closes — the jsdom stand-in for a
/// live SSE endpoint (sse.js readLoop stays pending; no reconnect timer).
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
    if (url.includes('/api/nodes')) {
      return jsonResponse({ nodes: [] });
    }
    // Longest suffixes first: /events?after=N must not fall into /sessions.
    if (url.includes('/events')) {
      return hangingStreamResponse();
    }
    if (url.includes('/seq')) {
      return jsonResponse({ seq: 0 });
    }
    if (url.includes('/prompt')) {
      return jsonResponse({ ok: true });
    }
    if (url.includes('/interrupt')) {
      return jsonResponse({ ok: true });
    }
    if (url === '/api/sessions' || url.startsWith('/api/sessions?')) {
      return method === 'POST' ? jsonResponse({ id: 's1' }) : jsonResponse({ sessions: [] });
    }
    return jsonResponse({});
  }));
};

// Deprecation gate: the X migration is only complete when rendering is silent.
const consoleLog = { error: [], warn: [] };
const record = (bucket) => (...args) => {
  consoleLog[bucket].push(args.map((a) => String(a)).join(' '));
};
const deprecationHits = () => consoleLog.error.concat(consoleLog.warn)
  .filter((line) => /deprecated/i.test(line));

beforeEach(() => {
  localStorage.clear();
  clearCredentials();
  setState({ page: 'chat', preselectNode: null, nodes: [], conn: 'init' });
  installRouter();
  vi.spyOn(console, 'error').mockImplementation(record('error'));
  vi.spyOn(console, 'warn').mockImplementation(record('warn'));
});

afterEach(() => {
  cleanup(); // unmount → ChatPanel aborts its hanging stream
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  expect(deprecationHits()).toEqual([]);
});

const turnsFixture = () => [
  { kind: 'text', role: 'user', text: '帮我跑一遍测试' },
  { kind: 'text', role: 'assistant', text: '好的，开始执行' },
  { kind: 'think', role: 'assistant', text: '先理解需求…' },
  { kind: 'tool', role: 'assistant', name: 'bash', input: 'npm test', output: 'Tests 32 passed', isError: true, durationMs: 1200, open: true },
  { kind: 'sys', text: 'status: streaming' },
];

describe('TranscriptView on Bubble.List', () => {
  it('renders turns as bubbles with tool error tag and usage footer', () => {
    const { container } = render(
      <TranscriptView
        turns={turnsFixture()}
        usage={{ input: 10, output: 5, total: 15, contextWindow: 100 }}
        status="streaming"
        error={null}
      />,
    );
    // Bubble.List landmark + one bubble per turn, user right-aligned.
    expect(container.querySelector('.ant-bubble-list')).toBeTruthy();
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(5);
    expect(container.querySelector('.ant-bubble-end')).toBeTruthy();
    expect(container.querySelectorAll('.ant-bubble-start')).toHaveLength(4);
    // Tool row keeps its error marker; usage footer text survives migration.
    expect(screen.getByText('error')).toBeTruthy();
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText(/▲ in 10/)).toBeTruthy();
    expect(screen.getByText(/上下文 15%/)).toBeTruthy();
    expect(screen.getByText('streaming…')).toBeTruthy();
  });
});

describe('ChatPanel full chain (Sender → signed POST → SSE)', () => {
  const mountChat = async () => {
    setCredentials('smoke-token', '');
    const { container } = render(<ChatPanel />);
    const textarea = await waitFor(() => {
      const el = container.querySelector('textarea.ant-sender-input');
      expect(el).toBeTruthy();
      return el;
    });
    return { container, textarea };
  };

  it('posts the typed prompt on Enter and clears the controlled input', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '你好，帮我跑个测试' } });
    });
    // Sender: Enter (no shift/modifiers) submits, exactly like production.
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      const promptHit = hits.find((h) => h.url.includes('/prompt'));
      expect(promptHit).toBeTruthy();
      expect(JSON.parse(promptHit.body).prompt).toBe('你好，帮我跑个测试');
    });
    // send() cleared the state → the controlled textarea is empty again.
    await waitFor(() => {
      expect(container.querySelector('textarea.ant-sender-input').value).toBe('');
    });
    // The prompt opened the signed event stream for the fresh session.
    expect(hits.some((h) => h.method === 'POST' && h.url === '/api/sessions')).toBe(true);
    // The events fetch lands right after the prompt ack — wait for it instead
    // of asserting synchronously (flaky under parallel CI load).
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/api/sessions/s1/events'))).toBe(true);
    });
  });

  it('swaps in the Sender stop button while busy and posts interrupt on click', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '长任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    // loading=true while the stream hangs → Sender shows the stop button.
    const stop = await waitFor(() => {
      const el = container.querySelector('.ant-sender-actions-btn-loading-button');
      expect(el).toBeTruthy();
      return el;
    });
    await act(async () => {
      fireEvent.click(stop);
    });
    await waitFor(() => {
      expect(hits.some((h) => h.method === 'POST' && h.url.includes('/interrupt'))).toBe(true);
    });
  });
});
