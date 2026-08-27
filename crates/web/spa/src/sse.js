// sse.js — SSE over fetch streaming. EventSource is unusable here: it cannot
// send the signature headers. Behavioral reference is the vanilla frontend's
// crates/web/src/assets/sse.js, whose reconnect decisions are mirrored:
//   * backoff 1s ×2 (cap 15s per product spec), reset on any received frame;
//   * max 5 consecutive failures, then a terminal 'failed' status;
//   * on reconnect, replay a bounded tail of the missed window through the
//     same reducer (the terminal frame of a finished run is always the head,
//     so a capped tail still converges — an uncapped replay of a fast stream
//     freezes the console, found by real-browser acceptance);
//   * a terminal done/error frame closes the stream for good.
//
// openStream({ path, sessionId, after, onFrame, onStatus, signal }) →
// { abort() }. `path` must NOT carry an ?after= param; this module owns the
// cursor, starting at `after` (0 = full replay).

import { apiGet, signFetch } from './api.js';

const BACKOFF_START_MS = 1000;
const BACKOFF_CAP_MS = 15000;
/// Max frames replayed on reconnect (see reconnectCursor).
const REPLAY_CAP_FRAMES = 400;
const MAX_ATTEMPTS = 5;

export function openStream({ path, sessionId, after, onFrame, onStatus, signal }) {
  const ctrl = new AbortController();
  let stopped = false;
  let attempts = 0;
  let backoff = BACKOFF_START_MS;
  let lastSeq = 0;
  let timer = null;

  const report = (status, info) => {
    if (typeof onStatus === 'function') {
      onStatus(status, info);
    }
  };
  const externalAbort = () => stop();
  if (signal) {
    if (signal.aborted) {
      stop();
    } else {
      signal.addEventListener('abort', externalAbort, { once: true });
    }
  }

  function stop() {
    if (stopped) {
      return;
    }
    stopped = true;
    clearTimeout(timer);
    ctrl.abort();
    if (signal) {
      signal.removeEventListener('abort', externalAbort);
    }
    report('closed');
  }

  /// Parse one SSE block (lines up to a blank line) → {event, data} | null.
  function parseBlock(block) {
    let event = 'message';
    const dataLines = [];
    for (const rawLine of block.split('\n')) {
      const line = rawLine.replace(/\r$/, '');
      if (!line || line.startsWith(':')) {
        continue; // keep-alive comment
      }
      if (line.startsWith('event:')) {
        event = line.slice(6).trim();
      } else if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).replace(/^ /, ''));
      } else if (line.startsWith('id:')) {
        const n = parseInt(line.slice(3).trim(), 10);
        if (Number.isFinite(n)) {
          lastSeq = n;
        }
      }
    }
    if (!dataLines.length) {
      return null;
    }
    const raw = dataLines.join('\n');
    let data;
    try {
      data = JSON.parse(raw);
    } catch {
      data = { raw };
    }
    if (data && typeof data.seq === 'number') {
      lastSeq = data.seq;
    }
    return { event, data };
  }

  function handleBlock(block) {
    const frame = parseBlock(block);
    if (!frame) {
      return;
    }
    attempts = 0; // any frame proves the stream is alive
    backoff = BACKOFF_START_MS;
    report('live');
    if (typeof onFrame === 'function') {
      onFrame(frame);
    }
    if (frame.event === 'done' || frame.event === 'error') {
      stop(); // terminal for the subscribed task: never reconnect
    }
  }

  async function readLoop(resp) {
    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    for (;;) {
      const { done, value } = await reader.read();
      if (stopped) {
        try {
          reader.cancel();
        } catch {
          /* already gone */
        }
        return;
      }
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      let sep;
      while ((sep = buffer.indexOf('\n\n')) >= 0) {
        const block = buffer.slice(0, sep);
        buffer = buffer.slice(sep + 2);
        handleBlock(block);
        if (stopped) {
          return;
        }
      }
    }
    // Server closed cleanly WITHOUT a terminal frame (proxy timeout, server
    // restart mid-run). Mirror of sse.js's error → tryReconnect path: retry
    // from the last cursor; the attempt cap ends pathological loops. (A clean
    // close is never the normal end — finished tasks carry a done frame.)
    scheduleReconnect();
  }

  /// Mirror of sse.js tryReconnect: every retry re-reads the persisted head
  /// via /api/sessions/:id/seq so a reconnect never replays the whole dialog
  /// from 0 (frames missed in the gap are covered by the done → transcript
  /// reload in chat.jsx). Never regress below the last cursor we actually saw.
  function scheduleReconnect() {
    if (stopped) {
      return;
    }
    if (attempts >= MAX_ATTEMPTS) {
      report('failed');
      stop();
      return;
    }
    const delay = Math.min(backoff, BACKOFF_CAP_MS);
    backoff *= 2;
    attempts += 1;
    report('reconnecting', { attempt: attempts, delay });
    timer = setTimeout(async () => {
      if (stopped) {
        return;
      }
      const after = await reconnectCursor();
      connect(after);
    }, delay);
  }

  /// Reconnect cursor: the run may have raced ahead while the link was down,
  /// so ask the store for the head — then cap how much of the missed window
  /// gets replayed (a fast stream can miss tens of thousands of delta frames
  /// and folding them all back in freezes the tab; a finished run's terminal
  /// frame IS the head, so the capped tail still converges).
  async function reconnectCursor() {
    if (!sessionId) {
      return lastSeq;
    }
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sessionId) + '/seq');
      const head = j && typeof j.seq === 'number' ? j.seq : 0;
      return Math.max(lastSeq, head - REPLAY_CAP_FRAMES);
    } catch {
      return lastSeq; // seq fetch failed: counts as another attempt on retry
    }
  }

  async function connect(after) {
    if (stopped) {
      return;
    }
    const pathAndQuery = path + (path.includes('?') ? '&' : '?') + 'after=' + after;
    try {
      const resp = await signFetch('GET', pathAndQuery, undefined, { signal: ctrl.signal });
      if (stopped) {
        return;
      }
      if (!resp.ok || !resp.body) {
        throw new Error('stream HTTP ' + resp.status);
      }
      report('open');
      await readLoop(resp);
    } catch (e) {
      if (stopped || (e && e.name === 'AbortError')) {
        return;
      }
      scheduleReconnect();
    }
  }


  connect(Number.isFinite(after) ? after : 0);

  return { abort: stop };
}
