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

it('keeps terminal receipts over delayed or duplicate lifecycle events', () => {
  const done = applyDagFrame(snapshot, frame(21, 'step_done', 60, {}));
  expect(done.steps[0]).toMatchObject({ status: 'done', at_ms: 60, error: null });
  expect(applyDagFrame(done, frame(21, 'step_done', 60, { ok: false }))).toBe(done);
  expect(applyDagFrame(done, frame(22, 'step_started', 60)).steps[0].status).toBe('done');
  expect(applyDagFrame(done, frame(23, 'step_done', 40, { ok: false })).steps[0].status).toBe('done');
});

it('applies run lifecycle and newest dynamic progress without changing other steps', () => {
  const initial = { head_seq: 0, execution_status: 'pending', steps: [
    { name: 'a', status: 'pending' }, { name: 'b', status: 'pending' },
  ] };
  const started = applyDagFrame(initial, frame(1, 'run_started', 10));
  expect(started.execution_status).toBe('running');
  const progress = applyDagFrame(started, frame(2, 'step_progress', 20, { instances: { done: 1 } }));
  expect(progress.steps[0].instances).toEqual({ done: 1 });
  const delayed = applyDagFrame(progress, frame(3, 'step_progress', 15, { instances: { done: 0 } }));
  expect(delayed.steps[0]).toBe(progress.steps[0]);
  const finished = applyDagFrame(delayed, frame(4, 'run_finished', 30, { status: 'error', error: 'failed' }));
  expect(finished).toMatchObject({ head_seq: 4, execution_status: 'error', execution_error: 'failed' });
  expect(finished.steps[1]).toBe(initial.steps[1]);
  expect(initial.execution_status).toBe('pending');
});
