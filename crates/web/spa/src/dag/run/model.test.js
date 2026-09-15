import { expect, it } from 'vitest';
import { applyDagFrame, checkedSnapshot } from './model.js';

const snapshot = { head_seq: 20, execution_status: 'running', steps: [{ name: 'a', status: 'done', at_ms: 50 }] };
const frame = (seq, event, at_ms, payload = {}) => ({ seq, event, data: { step: 'a', at_ms, payload } });

it('ignores replay, logs, and delayed starts while accepting a new attempt', () => {
  expect(applyDagFrame(snapshot, frame(10, 'step_started', 30))).toBe(snapshot);
  expect(applyDagFrame(snapshot, frame(21, 'step_log', 60))).toBe(snapshot);
  expect(applyDagFrame(snapshot, frame(21, 'step_started', 40)).steps[0].status).toBe('done');
  const retry = applyDagFrame(snapshot, frame(22, 'step_started', 60));
  expect(retry.steps[0].status).toBe('running');
  expect(snapshot.steps[0].status).toBe('done');
  const failed = applyDagFrame(retry, frame(23, 'step_done', 70, { ok: false, error: 'refused' }));
  expect(failed.steps[0]).toMatchObject({ status: 'error', error: 'refused' });
});

it('requires a valid snapshot watermark', () => {
  expect(checkedSnapshot(snapshot)).toBe(snapshot);
  for (const head_seq of [undefined, -1, '20', NaN]) expect(() => checkedSnapshot({ ...snapshot, head_seq })).toThrow('快照');
});
