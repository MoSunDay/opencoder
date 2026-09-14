import { afterEach, expect, it, vi } from 'vitest';
import { openStream } from './sse.js';
import { authFetch } from './api.js';
vi.mock('./api.js', () => ({ authFetch: vi.fn(), apiGet: vi.fn() }));
afterEach(() => { vi.useRealTimers(); vi.resetAllMocks(); });
const response = (text) => ({ ok: true, body: new ReadableStream({ start(controller) { controller.enqueue(new TextEncoder().encode(text)); controller.close(); } }) });
it('recovers an unexpected EOF without losing deltas and stops only on the server end marker', async () => {
  vi.useFakeTimers();
  authFetch.mockResolvedValueOnce(response('id: 1\nevent: step_log\ndata: {"text":"one"}\n\n'))
    .mockResolvedValueOnce(response('id: 1\nevent: step_log\ndata: {"text":"one"}\n\nid: 2\nevent: step_log\ndata: {"text":"two"}\n\nevent: stream_end\ndata: {"finished":true}\n\n'));
  const frames = [], statuses = [];
  openStream({ path: '/api/executions/a/events', executionHistory: true, requireEnd: true, onFrame: (f) => frames.push(f), onStatus: (s) => statuses.push(s) });
  await vi.advanceTimersByTimeAsync(1100);
  expect(authFetch.mock.calls[1][1]).toBe('/api/executions/a/events?after=1');
  expect(frames.map((f) => f.data.text)).toEqual(['one', 'two']);
  expect(statuses).toContain('reconnecting');
  expect(statuses.at(-1)).toBe('closed');
  await vi.advanceTimersByTimeAsync(16000);
  expect(authFetch).toHaveBeenCalledTimes(2);
});
it('retries a node transport error while preserving persisted execution errors', async () => {
  vi.useFakeTimers();
  authFetch.mockResolvedValueOnce(response('event: error\ndata: {"error":"node offline"}\n\n'))
    .mockResolvedValueOnce(response('id: 3\nevent: error\ndata: {"error":"step failed"}\n\nevent: stream_end\ndata: {"finished":true}\n\n'));
  const frames = [];
  openStream({ path: '/api/executions/a/events', executionHistory: true, requireEnd: true, onFrame: (f) => frames.push(f) });
  await vi.advanceTimersByTimeAsync(1100);
  expect(frames).toEqual([{ seq: 3, event: 'error', data: { error: 'step failed' } }]);
});
