import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./api.js', () => ({ authFetch: vi.fn(), apiGet: vi.fn() }));
import { authFetch, apiGet } from './api.js';
import { openStream } from './sse.js';

function response(text) {
  return new Response(new ReadableStream({ start(controller) {
    controller.enqueue(new TextEncoder().encode(text)); controller.close();
  } }), { status: 200 });
}
afterEach(() => { vi.useRealTimers(); vi.resetAllMocks(); });

it('a release reconnect replays from the last delivered cursor without skipping to the head', async () => {
  vi.useFakeTimers();
  authFetch.mockResolvedValueOnce(response('id: 7\nevent: step_log\ndata: {"text":"before"}\n\nid: 7\nevent: reconnect\ndata: release switch\n\n'))
    .mockResolvedValueOnce(response('id: 8\nevent: step_log\ndata: {"text":"after"}\n\nevent: stream_end\ndata: {"finished":true}\n\n'));
  apiGet.mockResolvedValue({ seq: 9000 });
  const frames = [];
  const stop = openStream({ path: '/api/executions/dag-a/events', sessionId: 'dag-a', executionHistory: true, onFrame: (frame) => frames.push(frame) });
  await vi.advanceTimersByTimeAsync(500);
  expect(authFetch).toHaveBeenCalledTimes(2);
  expect(String(authFetch.mock.calls[1][1])).toContain('after=7');
  expect(apiGet).not.toHaveBeenCalled();
  expect(frames.map((frame) => frame.seq)).toEqual([7, 8]);
  stop.abort();
});
