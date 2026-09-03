// sse.test.js — P0-2 contract: a synthetic `error` frame carrying `lag` (api.rs
// map_broadcast_result marks server-side consumer lag) must RECONNECT from the
// persisted head, while a real run `error` stays terminal. Before this test
// any error frame stopped the stream for good, so one slow tab froze the
// console on `error` while the run kept producing.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { openStream } from './sse.js';

const signFetchMock = vi.fn();
const apiGetMock = vi.fn();
vi.mock('./api.js', () => ({
  signFetch: (...a) => signFetchMock(...a),
  apiGet: (...a) => apiGetMock(...a),
}));

/// Minimal SSE response: enqueue the frames, then hold the connection open
/// (a real server keeps the stream alive between events).
function sseResponse(frames) {
  const enc = new TextEncoder();
  const payload = frames
    .map((f) => 'event: ' + f.event + '\ndata: ' + JSON.stringify(f.data || {}) + '\n\n')
    .join('');
  const body = new ReadableStream({
    start(controller) {
      controller.enqueue(enc.encode(payload));
    },
  });
  return { ok: true, status: 200, body };
}

/// A controllable live stream: frames are pushed after connect, mirroring a
/// real server that keeps the SSE connection open between events.
function liveStream() {
  const enc = new TextEncoder();
  let controller = null;
  const body = new ReadableStream({
    start(c) {
      controller = c;
    },
  });
  return {
    resp: { ok: true, status: 200, body },
    push(event, data) {
      controller.enqueue(enc.encode(
        'event: ' + event + '\ndata: ' + JSON.stringify(data || {}) + '\n\n',
      ));
    },
  };
}

function flush() {
  // Drain pending microtasks so readLoop/handleBlock run before assertions.
  return Promise.resolve().then(() => Promise.resolve()).then(() => Promise.resolve());
}

describe('sse lag contract', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    signFetchMock.mockReset();
    apiGetMock.mockReset();
  });

  it('reconnects from the seq head after a lag-marked error frame', async () => {
    signFetchMock.mockResolvedValueOnce(
      sseResponse([{ event: 'error', data: { error: 'event lag: 5 events dropped', lag: 5 } }]),
    );
    signFetchMock.mockResolvedValueOnce(sseResponse([{ event: 'done', data: {} }]));
    apiGetMock.mockResolvedValue({ seq: 42 });

    const frames = [];
    const statuses = [];
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: (f) => frames.push(f),
      onStatus: (s) => statuses.push(s),
    });

    await flush();
    await vi.advanceTimersByTimeAsync(0);
    await flush();
    expect(frames.map((f) => f.event)).toEqual(['error']);
    expect(statuses).not.toContain('closed');

    // Backoff fires → reconnectCursor (GET /seq) → second connect.
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(apiGetMock).toHaveBeenCalledWith('/api/sessions/s1/seq');
    expect(signFetchMock).toHaveBeenCalledTimes(2);
    expect(signFetchMock.mock.calls[1][1]).toContain('after=');

    // The second connection terminates normally.
    await vi.advanceTimersByTimeAsync(50);
    await flush();
    expect(statuses).toContain('closed');
    stream.abort();
  });

  it('retires the old connection on lag: post-lag frames drop, reconnects never stack', async () => {
    // F1 regression: the lag path used to scheduleReconnect WITHOUT retiring
    // the old readLoop. The server keeps the merged stream alive after a
    // Lagged recv error, so old + new streams delivered frames concurrently
    // (duplicate deltas) and repeated lag frames stacked connections.
    const connA = liveStream();
    const connB = liveStream();
    signFetchMock.mockResolvedValueOnce(connA.resp);
    signFetchMock.mockResolvedValueOnce(connB.resp);
    apiGetMock.mockResolvedValue({ seq: 100 });

    const frames = [];
    const statuses = [];
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: (f) => frames.push(f),
      onStatus: (s) => statuses.push(s),
    });

    await flush();
    // Two lag frames arrive back-to-back: the first retires the connection
    // and schedules exactly ONE reconnect; the second must be swallowed. The
    // delta behind them belongs to the retired stream and must never render.
    connA.push('error', { error: 'event lag: 5 events dropped', lag: 5 });
    connA.push('error', { error: 'event lag: 4 events dropped', lag: 4 });
    connA.push('text_delta', { text: 'ghost' });
    await flush();

    expect(frames.map((f) => f.event)).toEqual(['error']);
    expect(frames[0].data.lag).toBe(5);
    expect(frames.some((f) => f.data && f.data.text === 'ghost')).toBe(false);
    expect(statuses).not.toContain('closed'); // retired, not closed

    // Backoff fires → exactly one GET /seq → exactly one replacement connect.
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(apiGetMock).toHaveBeenCalledTimes(1);
    expect(signFetchMock).toHaveBeenCalledTimes(2);

    // The replacement stream carries the run to its terminal frame.
    connB.push('done', {});
    await flush();
    expect(frames.map((f) => f.event)).toEqual(['error', 'done']);
    expect(statuses).toContain('closed');
    stream.abort();
  });

  it('a real error frame (no lag marker) stays terminal', async () => {
    signFetchMock.mockResolvedValueOnce(sseResponse([{ event: 'error', data: { error: 'boom' } }]));
    const frames = [];
    const statuses = [];
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: (f) => frames.push(f),
      onStatus: (s) => statuses.push(s),
    });
    await flush();
    await vi.advanceTimersByTimeAsync(2000);
    await flush();
    expect(frames.map((f) => f.event)).toEqual(['error']);
    expect(statuses).toContain('closed');
    expect(signFetchMock).toHaveBeenCalledTimes(1); // never reconnected
    stream.abort();
  });

  it('a done frame still terminates the stream', async () => {
    signFetchMock.mockResolvedValueOnce(sseResponse([{ event: 'done', data: {} }]));
    const statuses = [];
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: () => {},
      onStatus: (s) => statuses.push(s),
    });
    await flush();
    await vi.advanceTimersByTimeAsync(50);
    await flush();
    expect(statuses).toContain('closed');
    stream.abort();
  });
});

describe('sse resync dedup + onResync watermark (round-2 #5)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    signFetchMock.mockReset();
    apiGetMock.mockReset();
  });

  /// sseResponse with `id:` lines — the persisted row seq rides the id line.
  function sseResponseIds(frames) {
    const enc = new TextEncoder();
    const payload = frames
      .map((f) => 'event: ' + f.event + '\n' + (f.id ? 'id: ' + f.id + '\n' : '') + 'data: ' + JSON.stringify(f.data || {}) + '\n\n')
      .join('');
    return {
      ok: true,
      status: 200,
      body: new ReadableStream({
        start(controller) {
          controller.enqueue(enc.encode(payload));
        },
      }),
    };
  }

  it('exposes the id-line seq on the frame and drops repeats at/below the delivered seq', async () => {
    signFetchMock.mockResolvedValueOnce(sseResponseIds([
      { event: 'text_delta', data: { text: 'a' }, id: 5 },
      { event: 'text_delta', data: { text: 'a' }, id: 5 }, // exact repeat
      { event: 'text_delta', data: { text: 'b' }, id: 4 }, // below watermark
      { event: 'text_delta', data: { text: 'c' }, id: 6 },
      { event: 'text_delta', data: { text: 'live' } }, // no seq: never deduped
    ]));
    const frames = [];
    const statuses = [];
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: (f) => frames.push(f),
      onStatus: (s) => statuses.push(s),
    });
    await flush();
    expect(frames.map((f) => f.seq)).toEqual([5, 6, null]);
    expect(frames.map((f) => f.data.text)).toEqual(['a', 'c', 'live']);
    expect(statuses.filter((s) => s === 'live')).toHaveLength(5); // repeats still prove liveness
    stream.abort();
  });

  it('a lag re-sync with onResync reconnects above the returned floor and skips the legacy /seq fetch', async () => {
    const connA = liveStream();
    const connB = liveStream();
    signFetchMock.mockResolvedValueOnce(connA.resp).mockResolvedValueOnce(connB.resp);
    const frames = [];
    const resyncArgs = [];
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: (f) => frames.push(f),
      onResync: async (lastSeq) => {
        resyncArgs.push(lastSeq);
        return 42; // the app rebuilt its state from the snapshot at seq 42
      },
    });
    await flush();
    connA.push('error', { error: 'event lag: 5 events dropped', lag: 5 });
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(resyncArgs).toEqual([0]);
    expect(apiGetMock).not.toHaveBeenCalled(); // onResync owns the cursor
    expect(signFetchMock).toHaveBeenCalledTimes(2);
    expect(signFetchMock.mock.calls[1][1]).toContain('after=42');
    // The replacement stream runs to its terminal frame normally.
    connB.push('done', {});
    await flush();
    expect(frames.map((f) => f.event)).toEqual(['error', 'done']);
    stream.abort();
  });

  it('a throwing onResync falls back to the capped legacy cursor', async () => {
    const connA = liveStream();
    const connB = liveStream();
    signFetchMock.mockResolvedValueOnce(connA.resp).mockResolvedValueOnce(connB.resp);
    apiGetMock.mockResolvedValue({ seq: 1000 });
    const stream = openStream({
      path: '/api/sessions/s1/events',
      sessionId: 's1',
      after: 0,
      onFrame: () => {},
      onResync: async () => {
        throw new Error('snapshot unavailable');
      },
    });
    await flush();
    connA.push('error', { error: 'event lag: 9 events dropped', lag: 9 });
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(apiGetMock).toHaveBeenCalledTimes(1); // legacy path re-read /seq
    expect(signFetchMock).toHaveBeenCalledTimes(2);
    expect(signFetchMock.mock.calls[1][1]).toContain('after=600'); // max(0, 1000-400)
    stream.abort();
  });
});
