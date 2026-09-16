import { describe, expect, it } from 'vitest';
import { STEP_KIND_LABEL, finishedOf, isAgentKind, outputRows } from './model.js';

// Flat node-side wasm capture frame.
const out = (seq, stream, text) => ({ seq, event: 'step_output', data: { step: 'build', stream, text, at_ms: seq } });
// Nested run-session mirror frame (step_log envelope).
const nested = (seq, event, data) => ({ seq, event: 'step_log', data: { kind: 'step_log', step: 'build', payload: { event, data }, at_ms: seq } });

describe('dag step projections', () => {
  it('merges adjacent stdout fragments and never merges across streams', () => {
    const rows = outputRows([out(1, 'stdout', 'comp'), out(2, 'stdout', 'iling'), out(3, 'stderr', 'boom'), out(4, 'stdout', 'ok')]);
    expect(rows.map((r) => [r.stream, r.label, r.text])).toEqual([
      ['stdout', 'stdout', 'compiling'],
      ['stderr', 'stderr', 'boom'],
      ['stdout', 'stdout', 'ok'],
    ]);
    // The merged row keeps the first fragment's seq/at for keys and sorting.
    expect(rows[0].seq).toBe(1);
    expect(rows[0].at).toBe(1);
  });
  it('reads nested step_log mirrors and ignores non-output kinds', () => {
    const rows = outputRows([
      nested(1, 'stdout', { text: 'a' }),
      nested(2, 'stderr', 'b'),
      nested(3, 'text_delta', { text: 'say' }),
      nested(4, 'text_delta', { text: ' more' }),
      nested(5, 'tool_start', { name: 'bash' }),
      { seq: 6, event: 'status', data: { status: 'running' } },
      { seq: 7, event: 'step_finished', data: { status: 'done' } },
    ]);
    expect(rows.map((r) => [r.stream, r.label, r.text])).toEqual([
      ['stdout', 'stdout', 'a'],
      ['stderr', 'stderr', 'b'],
      ['text_delta', '输出', 'say more'],
    ]);
  });
  it('filters rows with a case-insensitive substring query', () => {
    const frames = [out(1, 'stdout', 'Hello World'), out(2, 'stderr', 'BOOM')];
    expect(outputRows(frames, 'hello').map((r) => r.text)).toEqual(['Hello World']);
    expect(outputRows(frames, 'oom').map((r) => r.stream)).toEqual(['stderr']);
    expect(outputRows(frames, 'missing')).toEqual([]);
  });
  it('reports the last terminal receipt from finishedOf', () => {
    expect(finishedOf([])).toBeNull();
    expect(finishedOf([out(1, 'stdout', 'x')])).toBeNull();
    const done = { status: 'done', error: null, started_at_ms: 1, finished_at_ms: 2 };
    const frames = [
      { seq: 2, event: 'step_finished', data: { status: 'running' } },
      { seq: 3, event: 'step_finished', data: done },
    ];
    expect(finishedOf(frames)).toBe(done);
  });
  it('labels step kinds and detects agent steps', () => {
    expect(STEP_KIND_LABEL).toEqual({ wasm: 'Wasm 步骤', agent: 'Agent 步骤' });
    expect(isAgentKind('agent')).toBe(true);
    expect(isAgentKind('wasm')).toBe(false);
    expect(isAgentKind(undefined)).toBe(false);
  });
});
