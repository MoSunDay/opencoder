import { afterEach, describe, expect, it, vi } from 'vitest';
import { openStream } from './sse.js';
import { authFetch } from './api.js';
vi.mock('./api.js', () => ({ authFetch: vi.fn(), apiGet: vi.fn() }));
afterEach(() => vi.resetAllMocks());

function response(frames) {
  return { ok: true, body: new ReadableStream({ start(controller) {
    controller.enqueue(new TextEncoder().encode(frames.map((frame, i) => `id: ${i + 1}\nevent: ${frame.event}\ndata: ${JSON.stringify(frame.data || {})}\n\n`).join('')));
    controller.close();
  } }) };
}

describe('fleet execution event history', () => {
  it('replays past historical terminal frames and closes only at EOF', async () => {
    authFetch.mockResolvedValue(response([
      { event: 'text_delta', data: { text: 'one' } }, { event: 'done' },
      { event: 'error', data: { error: 'old error' } }, { event: 'text_delta', data: { text: 'two' } }, { event: 'done' },
    ]));
    const frames = [];
    await new Promise((resolve) => openStream({ path: '/api/executions/a/events', executionHistory: true,
      onFrame: (frame) => frames.push(frame), onStatus: (status) => { if (status === 'closed') resolve(); },
    }));
    expect(frames.map((frame) => frame.event)).toEqual(['text_delta', 'done', 'error', 'text_delta', 'done']);
    expect(frames.map((frame) => frame.seq)).toEqual([1, 2, 3, 4, 5]);
    expect(authFetch).toHaveBeenCalledTimes(1);
  });
  it('preserves the single-run terminal contract for existing subscribers', async () => {
    authFetch.mockResolvedValue(response([{ event: 'done' }, { event: 'text_delta', data: { text: 'later' } }]));
    const frames = [];
    await new Promise((resolve) => openStream({ path: '/api/sessions/a/events', onFrame: (frame) => frames.push(frame),
      onStatus: (status) => { if (status === 'closed') resolve(); },
    }));
    expect(frames.map((frame) => frame.event)).toEqual(['done']);
  });
});
