// Live execution replay uses the same reducer as chat. Fleet events are
// persisted and sequenced, so reconnect from the exact cursor: never skip
// deltas, nor fold the message snapshot and its events into the same state.
import { emptyStream, reduceFrame } from '../../reduce.js';

const ACTIVITY = new Set(['text_delta', 'reasoning_delta', 'tool_start', 'tool_end', 'steer_consumed', 'queue_consumed']);

export function reduceExecutionFrame(state, frame, now) {
  if (Number.isFinite(frame.seq) && frame.seq <= (state.applySeq ?? -1)) return state;
  const base = ACTIVITY.has(frame.event) ? { ...state, status: 'streaming' } : state;
  const next = reduceFrame(base, frame, now);
  // Same bounded window as persisted history. Throttle byte accounting on
  // delta frames so token streaming does not stringify megabytes per token.
  const checkSize = !['text_delta', 'reasoning_delta'].includes(frame.event)
    || !Number.isFinite(frame.seq) || frame.seq % 64 === 0 || next.turns.length > 100;
  if (!checkSize) return next;
  const turns = next.turns.slice();
  let bytes = JSON.stringify(turns).length;
  let trimmed = state.trimmed || false;
  while (turns.length > 1 && (turns.length > 100 || bytes > 4 * 1024 * 1024)) {
    bytes -= JSON.stringify(turns.shift()).length;
    trimmed = true;
  }
  return { ...next, turns, trimmed };
}

export function initialExecutionTranscript() {
  return { ...emptyStream(), trimmed: false };
}
