// @vitest-environment jsdom
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const get = vi.hoisted(() => vi.fn());
const open = vi.hoisted(() => vi.fn());
vi.mock('../../api.js', () => ({ apiGet: get }));
vi.mock('../../sse.js', () => ({ openStream: open }));
import { useDagProgress } from './useDagProgress.js';
const snapshot = (head_seq, status = 'running') => ({ head_seq, execution_status: status, steps: [{ name: 'a', status }] });
beforeEach(() => { get.mockReset(); open.mockReset().mockImplementation(() => ({ abort: vi.fn() })); });

it('resynchronizes from a fresh snapshot and ignores an in-flight stale response', async () => {
  let reply;
  get.mockResolvedValueOnce(snapshot(10)).mockImplementationOnce(() => new Promise((resolve) => { reply = resolve; }));
  const hook = renderHook(() => useDagProgress({ id: 'one', status: 'running' }));
  await waitFor(() => expect(open).toHaveBeenCalledOnce());
  const stream = open.mock.calls[0][0];
  let resync;
  act(() => { resync = stream.onResync(); });
  act(() => stream.onFrame({ seq: 12, event: 'step_done', data: { step: 'a', at_ms: 12, payload: { ok: true } } }));
  await act(async () => { reply(snapshot(11)); await resync; });
  expect(hook.result.current.snapshot.steps[0].status).toBe('done');
  expect(await resync).toBe(12);
  get.mockResolvedValue(snapshot(20, 'done'));
  await act(async () => { await stream.onResync(); });
  expect(hook.result.current.snapshot.head_seq).toBe(20);
  expect(open.mock.results[0].value.abort).toHaveBeenCalled();
});

it('clears the previous run, aborts its stream, and reports invalid snapshots', async () => {
  get.mockResolvedValueOnce(snapshot(10)).mockResolvedValueOnce({ steps: [] });
  const hook = renderHook(({ id }) => useDagProgress({ id }), { initialProps: { id: 'one' } });
  await waitFor(() => expect(open).toHaveBeenCalledOnce());
  const old = open.mock.calls[0][0];
  hook.rerender({ id: 'two' });
  await waitFor(() => expect(hook.result.current.error).toContain('快照'));
  expect(hook.result.current.snapshot).toBeNull();
  act(() => old.onFrame({ seq: 99, event: 'run_finished', data: { payload: { status: 'done' } } }));
  expect(hook.result.current.snapshot).toBeNull();
  expect(open.mock.results[0].value.abort).toHaveBeenCalledOnce();
});

it('does not replay running when run_finished precedes the journal update', async () => {
  get.mockResolvedValueOnce(snapshot(10)).mockResolvedValue(snapshot(12));
  const hook = renderHook(() => useDagProgress({ id: 'finishing' }));
  await waitFor(() => expect(open).toHaveBeenCalledOnce());
  await act(async () => open.mock.calls[0][0].onFrame({ seq: 12, event: 'run_finished', data: { payload: { status: 'done' } } }));
  expect(hook.result.current.snapshot.execution_status).toBe('done');
  get.mockResolvedValue(snapshot(13));
  await act(async () => { await open.mock.calls[0][0].onResync(); });
  expect(hook.result.current.snapshot.execution_status).toBe('running');
});
