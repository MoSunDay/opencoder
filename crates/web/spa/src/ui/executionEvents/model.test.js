import { describe, expect, it } from 'vitest';
import { appendLog, initialLogs, logEntry, logRows, ROW_LIMIT } from './model.js';
const frame = (seq, step, event, text) => ({ seq, event: 'step_log', data: { step, at_ms: 1, payload: { event, data: { text } } } });
describe('execution log projection', () => {
  it('deduplicates resumed events and bounds retained history without regressing the cursor', () => {
    let state = initialLogs();
    for (let i = 1; i <= ROW_LIMIT + 1; i += 1) state = appendLog(state, frame(i, 'a', 'stdout', 'x'));
    expect(state.trimmed).toBe(true);
    expect(state.frames).toHaveLength(ROW_LIMIT);
    expect(state.frames[0].seq).toBe(2);
    expect(appendLog(state, frame(1, 'a', 'stdout', 'duplicate'))).toBe(state);
    expect(state.cursor).toBe(ROW_LIMIT + 1);
  });
  it('joins token fragments, keeps interleaved steps distinct and searches the joined output', () => {
    const frames = [frame(1, 'a', 'text_delta', 'hello '), frame(2, 'a', 'text_delta', 'world'), frame(3, 'b', 'stderr', 'failed')];
    expect(logRows(frames).map((r) => r.text)).toEqual(['hello world', 'failed']);
    expect(logRows(frames, 'a', 'hello world')).toHaveLength(1);
    expect(logRows(frames, 'a', 'failed')).toHaveLength(0);
  });
  it('exposes oversized records for chunk reading and handles ordinary execution events', () => {
    const marker = { omitted: true, read_via: 'event_payload' };
    expect(logEntry({ seq: 5, event: 'step_log', data: marker }).marker).toBe(marker);
    expect(logRows([{ seq: 1, event: 'tool_end', data: { name: 'shell', output: 'exit=0' } }])[0].text).toBe('shell\nexit=0');
  });
  it('projects node-side step_output frames onto merged stdout/stderr rows', () => {
    const out = (seq, stream, text) => ({ seq, event: 'step_output', data: { step: 'a', stream, text, at_ms: seq } });
    expect(logEntry(out(1, 'stderr', 'boom'))).toMatchObject({ event: 'stderr', label: 'stderr', step: 'a', text: 'boom' });
    const rows = logRows([out(1, 'stdout', 'he'), out(2, 'stdout', 'llo'), out(3, 'stderr', 'boom'), out(4, 'stdout', 'ok')]);
    expect(rows.map((r) => [r.event, r.text])).toEqual([['stdout', 'hello'], ['stderr', 'boom'], ['stdout', 'ok']]);
  });
});
