import { afterEach, beforeEach, expect, vi } from 'vitest';
import { cleanup } from '@testing-library/react';
import { clearCredentials, setState } from '../store.js';

export const chatTestState = { hits: [], sessionSnapshots: {}, seqHead: 0, liveEventCtl: null };

const jsonResponse = (body) => Promise.resolve({
  ok: true,
  status: 200,
  json: () => Promise.resolve(body),
});

const controlledStreamResponse = () => new Response(
  new ReadableStream({
    start(controller) {
      chatTestState.liveEventCtl = controller;
    },
  }),
  { status: 200, headers: { 'content-type': 'text/event-stream' } },
);

const installRouter = () => {
  chatTestState.hits = [];
  vi.stubGlobal('fetch', vi.fn((input, opts = {}) => {
    const url = typeof input === 'string' ? input : String((input && input.url) || '');
    const method = String(opts.method || 'GET').toUpperCase();
    chatTestState.hits.push({ method, url, body: opts.body || '' });
    if (url.includes('/events')) {
      return controlledStreamResponse();
    }
    if (url.includes('/tasks') && method === 'POST') {
      return jsonResponse({ task_id: 't1', session_id: 'rs1' });
    }
    if (url.includes('/api/nodes')) {
      return jsonResponse({ nodes: [{ id: 'node-1', name: 'Worker', online: true, kinds: ['agent'], snapshot: { ready: true } }] });
    }
    if (url.includes('/seq')) {
      return jsonResponse({ seq: chatTestState.seqHead });
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
      return jsonResponse(chatTestState.sessionSnapshots[url.slice('/api/sessions/'.length)] || {});
    }
    return jsonResponse({});
  }));
};

const consoleLog = { error: [], warn: [] };
const record = (bucket) => (...args) => {
  consoleLog[bucket].push(args.map((a) => String(a)).join(' '));
};
const deprecationHits = () => consoleLog.error.concat(consoleLog.warn)
  .filter((line) => /deprecated/i.test(line));

beforeEach(() => {
  chatTestState.liveEventCtl = null;
  chatTestState.seqHead = 0;
  chatTestState.sessionSnapshots = {};
  consoleLog.error.length = 0; // keep spy identity; per-test deprecation gate
  consoleLog.warn.length = 0;
  localStorage.clear();
  clearCredentials();
  setState({ page: 'chat', preselectNode: 'node-1', nodes: [], conn: 'init' });
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

