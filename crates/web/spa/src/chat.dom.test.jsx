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

// Per-session bodies for GET /api/sessions/:id — the done / transcript_reset
// reload paths. Default {} (empty snapshot → streamed turns kept); a test
// seeds an entry to simulate what the store has (or has NOT) recorded.
let sessionSnapshots = {};

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

/// Same, but the controller is captured so a test can push SSE blocks into
/// the "live" stream (terminal error frames, lag re-sync, …).
let liveEventCtl = null;
const controlledStreamResponse = () => new Response(
  new ReadableStream({
    start(controller) {
      liveEventCtl = controller;
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
    // Live-stream and dispatch routes first: the broad /api/nodes catch-all
    // below used to shadow them, so node-task streams
    // (/api/nodes/tasks/:id/events) and node dispatch POSTs
    // (/api/nodes/:id/tasks) answered { nodes: [] } and a remote stream
    // could never be driven from a test. /events still precedes /sessions
    // so /events?after=N never falls into the sessions rule.
    if (url.includes('/events')) {
      return controlledStreamResponse();
    }
    if (url.includes('/tasks') && method === 'POST') {
      return jsonResponse({ task_id: 't1', session_id: 'rs1' });
    }
    if (url.includes('/api/nodes')) {
      return jsonResponse({ nodes: [] });
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
    if (/^\/api\/sessions\/[^/]+$/.test(url)) {
      return jsonResponse(sessionSnapshots[url.slice('/api/sessions/'.length)] || {});
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
  liveEventCtl = null;
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

  it('snapshots the /seq head BEFORE posting the prompt (lost-frame race)', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'race' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    // Wait for the whole chain (session create → seq → prompt ack → stream),
    // then assert ORDERING via hits: entries are pushed synchronously, so
    // findIndex is a faithful call-order record.
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/api/sessions/s1/events'))).toBe(true);
    });
    const promptIdx = hits.findIndex((h) => h.method === 'POST' && h.url.includes('/prompt'));
    const seqIdx = hits.findIndex((h) => h.method === 'GET' && /\/api\/sessions\/[^/]+\/seq/.test(h.url));
    const eventsIdx = hits.findIndex((h) => h.url.includes('/api/sessions/s1/events'));
    // If the /seq head is fetched AFTER the prompt POST, events the run emits
    // between the ack and that late fetch get seq ≤ head and are never
    // replayed on the stream — this turn's first frames are lost forever.
    expect(promptIdx).toBeGreaterThan(-1);
    expect(seqIdx).toBeGreaterThan(-1);
    expect(seqIdx).toBeLessThan(promptIdx);
    expect(eventsIdx).toBeGreaterThan(promptIdx);
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

  it('releases the composer on a terminal error frame (busy must not latch)', async () => {
    // F4: only status==='done' used to reset busy, so a run ending in error
    // left the Sender loading forever and questionModal polling a dead stream.
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '会失败的任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();
    });
    await waitFor(() => {
      expect(liveEventCtl).toBeTruthy();
    });
    const enc = new TextEncoder();
    await act(async () => {
      liveEventCtl.enqueue(enc.encode(
        'event: error\ndata: ' + JSON.stringify({ error: 'boom' }) + '\n\n',
      ));
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeFalsy();
    });
  });

  it('keeps the typed input when a remote run is already busy', async () => {
    // Composite busy gate: while a remote task streams, Enter must neither
    // dispatch a second task nor clear the composer. The @ant-design/x
    // Sender itself refuses onSubmit while loading (Sender.js triggerSend
    // `!loading`), and send()'s own busy guards back that up — the F3 fix
    // moved setInput('') behind those guards so the remote-busy early return
    // can never silently swallow the typed prompt.
    setState({ page: 'chat', preselectNode: 'node-1', nodes: [], conn: 'init' });
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '第一个远程任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/nodes/node-1/tasks'))).toBe(true);
    });
    expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();

    // Second prompt while the remote run streams: input must survive.
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '第二个输入不能丢' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(container.querySelector('textarea.ant-sender-input').value).toBe('第二个输入不能丢');
    });
    expect(hits.filter((h) => h.url.includes('/tasks') && h.method === 'POST')).toHaveLength(1);
  });

  it('releases the composer when a FIRST remote dispatch reaches a terminal frame (no dialog selected yet)', async () => {
    // Fresh-remote busy latch: sendRemote only used to backfill dialogSel
    // when one was already selected, so the terminal-frame effect
    // early-returned on !dialogSel — the Sender stayed loading after
    // done/error until the user clicked some dialog. Both halves are pinned:
    // the terminal frame releases busy, and the backfilled selection makes
    // the done → store reload fire (that GET only happens when dialogSel is
    // set — removing the sendRemote backfill turns the last assertion red).
    setState({ page: 'chat', preselectNode: 'node-1', nodes: [], conn: 'init' });
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '首个远程任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/nodes/node-1/tasks'))).toBe(true);
    });
    expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();

    const enc = new TextEncoder();
    await act(async () => {
      liveEventCtl.enqueue(enc.encode('event: done\ndata: {}\n\n'));
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeFalsy();
    });
    // dialogSel was backfilled to 'rs1' → the done path reloads that session
    // from the store (fetch mock answers {} → streamed turns are kept).
    await waitFor(() => {
      expect(hits.some((h) => h.url === '/api/sessions/rs1')).toBe(true);
    });
  });
});

// TUI parity for the two composer/transcript gaps: the optimistic user echo
// on a fresh local submit (push_user — no waiting on any server frame) and
// the pending_turn_echo re-push after a transcript_reset rebuild whose store
// snapshot has not recorded the echo yet (rebuild_after_reset).
describe('ChatPanel optimistic echo & transcript_reset rebuild', () => {
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

  it('renders the optimistic user bubble right after a fresh local submit (no frames yet)', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '马上开始' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    // A fresh run carries NO steer/queue echo frame, so the optimistic echo
    // is the run's only user anchor (TUI push_user): it must render before
    // any stream frame — injected via startStream's initialTurns so the
    // stream reset cannot swallow it.
    await waitFor(() => {
      const bubble = container.querySelector('.ant-bubble-end');
      expect(bubble).toBeTruthy();
      expect(bubble.textContent).toContain('马上开始');
    });
    // No terminal frame arrived yet → still streaming, composer busy.
    expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();
  });

  it('renders no optimistic bubble for a bare control command submit', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '/act' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    // consumedEchoText('/act') === '' — a bare control command echoes nothing
    // (the runner applies it inline), so the opened stream renders no bubble.
    await waitFor(() => {
      expect(hits.some((h) => h.url.includes('/events'))).toBe(true);
    });
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(0);
  });

  it('re-pushes the pending echo after transcript_reset when the store snapshot lacks it', async () => {
    // The /act_clear_context <tail> scenario (TUI rebuild_after_reset): the
    // compound submission echoes its tail locally; the runner applies
    // ClearContext mid-run and the transcript resets, but the snapshot the
    // rebuild re-fetches has NOT recorded the tail yet — without the re-push
    // (ensurePendingEcho on pendingEcho) the rebuilt transcript ends at the
    // folded context and the running turn loses its user anchor.
    sessionSnapshots.s1 = {
      messages: [{ role: 'assistant', blocks: [{ kind: 'text', text: '压缩后的上下文' }] }],
    };
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '/act_clear_context 收尾总结' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    // Optimistic echo = the compound tail only; the command head never echoes.
    await waitFor(() => {
      const bubble = container.querySelector('.ant-bubble-end');
      expect(bubble).toBeTruthy();
      expect(bubble.textContent).toContain('收尾总结');
    });
    await waitFor(() => {
      expect(liveEventCtl).toBeTruthy();
    });
    const enc = new TextEncoder();
    await act(async () => {
      liveEventCtl.enqueue(enc.encode('event: transcript_reset\ndata: {}\n\n'));
    });
    // The rebuild fetched the assistant-only snapshot and re-pushed the echo:
    // the rebuilt transcript ENDS with the user bubble.
    await waitFor(() => {
      expect(hits.some((h) => h.url === '/api/sessions/s1')).toBe(true);
    });
    await waitFor(() => {
      const ends = container.querySelectorAll('.ant-bubble-end');
      expect(ends).toHaveLength(1);
      expect(ends[0].textContent).toContain('收尾总结');
    });
    expect(screen.getByText('压缩后的上下文')).toBeTruthy();
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(2);
  });
});
