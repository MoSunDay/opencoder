// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { useTranscriptStream } from '../useTranscriptStream.js';
import { emptyStream } from '../../reduce.js';
import { apiGet, authFetch } from '../../api.js';

vi.mock('../../api.js', () => ({ apiGet: vi.fn(), authFetch: vi.fn() }));
const end = 'event: stream_end\ndata: {"finished":true}\n\n';
const response = (text = end) => ({ ok: true, body: new ReadableStream({ start(controller) {
  controller.enqueue(new TextEncoder().encode(text)); controller.close();
} }) });
const refs = { streamRef: { current: null }, aliveRef: { current: true },
  selectionRef: { current: { node: 'node-a', kind: 'operator', dialog: 's1' } } };

function useChatStream() {
  const [stream, setStream] = useState(emptyStream);
  const [busy, setBusy] = useState(true);
  const [connecting, setConnecting] = useState(true);
  const actions = useTranscriptStream({ ...refs, setStream, setBusy, setConnecting, setQueueVersion: vi.fn() });
  return { ...actions, stream, busy, connecting };
}

beforeEach(() => {
  refs.streamRef.current = null;
  refs.selectionRef.current = { node: 'node-a', kind: 'operator', dialog: 's1' };
  authFetch.mockResolvedValue(response());
  apiGet.mockImplementation(async (path) => path.includes('/executions/')
    ? { execution: { status: 'idle' } }
    : { draining: false, messages: [
      { role: 'user', blocks: [{ kind: 'text', text: 'hello' }] },
      { role: 'assistant', blocks: [{ kind: 'text', text: 'saved answer' }] },
    ] });
});
afterEach(() => { refs.streamRef.current?.abort(); vi.resetAllMocks(); });

it('loads the saved answer and releases waiting when the only event is stream_end', async () => {
  const { result } = renderHook(useChatStream);
  await act(() => result.current.openSessionStream('s1', 0, [{ kind: 'text', role: 'user', text: 'hello' }]));
  await waitFor(() => expect(result.current.stream.status).toBe('done'));
  expect(result.current.stream.turns.filter((turn) => turn.role === 'user')).toHaveLength(1);
  expect(result.current.stream.turns.some((turn) => turn.text === 'saved answer')).toBe(true);
  expect(result.current.connecting).toBe(false);
  expect(result.current.busy).toBe(false);
});

it('shows an initialization error even when no session transcript was created', async () => {
  apiGet.mockImplementation(async (path) => {
    if (path.includes('/executions/')) return { execution: { status: 'error' }, error: 'runtime could not start' };
    throw new Error('session not found');
  });
  const { result } = renderHook(useChatStream);
  await act(() => result.current.openSessionStream('s1', 0, []));
  await waitFor(() => expect(result.current.stream.error).toBe('runtime could not start'));
  expect(result.current.stream.status).toBe('error');
  expect(result.current.connecting).toBe(false);
  expect(result.current.busy).toBe(false);
});

it('shows a result-read failure instead of leaving the composer busy', async () => {
  apiGet.mockRejectedValue(new Error('node offline'));
  const { result } = renderHook(useChatStream);
  await act(() => result.current.openSessionStream('s1', 0, []));
  await waitFor(() => expect(result.current.stream.status).toBe('error'));
  expect(result.current.stream.error).toContain('node offline');
  expect(result.current.busy).toBe(false);
  expect(result.current.connecting).toBe(false);
});

it('does not let a late completion replace a new subscription to the same conversation', async () => {
  let finish;
  apiGet.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const { result } = renderHook(useChatStream);
  await act(() => result.current.openSessionStream('s1', 0, []));
  await waitFor(() => expect(finish).toBeTypeOf('function'));
  authFetch.mockResolvedValue({ ok: true, body: new ReadableStream({ start() {} }) });
  await act(() => result.current.openSessionStream('s1', 10, [{ kind: 'text', role: 'user', text: 'next prompt' }]));
  await act(async () => finish({ execution: { status: 'error' }, error: 'old failure' }));
  expect(result.current.stream.status).toBe('streaming');
  expect(result.current.stream.turns[0].text).toBe('next prompt');
  expect(result.current.stream.error).toBeNull();
});
