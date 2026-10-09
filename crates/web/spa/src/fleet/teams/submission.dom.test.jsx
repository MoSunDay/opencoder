// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook } from '@testing-library/react';
import { apiGet, apiPost } from '../../api.js';
import { useTeamSubmission } from './useSubmission.js';
import { receiptOutcome, SUBMIT_WAIT_MS, RECEIPT_POLL_MS } from './submission.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
beforeEach(() => vi.useFakeTimers());
afterEach(() => { cleanup(); vi.resetAllMocks(); vi.useRealTimers(); });
const values = { prompt: '只回复 pong', node: '' };
const team = { name: 'sample' };
const index = (id) => ({ id, kind: 'team', status: 'running' });
const receipt = (id, phase = 'prepared') => ({ id, phase, receipt: phase === 'accepted' ? { status: 202, body: index(id) } : null });
const idFrom = (path) => path.split('/').at(-2);

it('leaves a hung POST after 15 seconds and automatically opens the matching accepted receipt without another POST', async () => {
  let late;
  apiPost.mockImplementation(() => new Promise((resolve) => { late = resolve; }));
  apiGet.mockImplementation(async (path) => receipt(idFrom(path)));
  const accepted = vi.fn();
  const view = renderHook(() => useTeamSubmission(team, accepted));
  act(() => { view.result.current.submit(values); });
  await act(async () => {});
  expect(view.result.current.busy).toBe(true);
  const id = apiPost.mock.calls[0][1].id;
  expect(view.result.current.attempt.id).toBe(id);
  await act(async () => { await vi.advanceTimersByTimeAsync(SUBMIT_WAIT_MS); });
  expect(view.result.current.busy).toBe(false);
  expect(view.result.current.confirming).toBe(true);
  expect(view.result.current.registered).toBe(true);
  expect(apiPost.mock.calls[0][2].signal.aborted).toBe(true);
  expect(accepted).not.toHaveBeenCalled();
  apiGet.mockImplementation(async (path) => receipt(idFrom(path), 'accepted'));
  await act(async () => { await vi.advanceTimersByTimeAsync(RECEIPT_POLL_MS); });
  expect(accepted).toHaveBeenCalledWith({ ...index(id), name: 'sample' });
  await act(async () => { late(index(id)); });
  expect(accepted).toHaveBeenCalledTimes(1);
  expect(apiPost).toHaveBeenCalledTimes(1);
});

it('keeps an uncertain request and its input when closed and resumed, and aborts stale reads', async () => {
  apiPost.mockRejectedValue(Object.assign(new Error('gateway timeout'), { status: 504 }));
  let finish;
  apiGet.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const accepted = vi.fn();
  const view = renderHook(({ team }) => useTeamSubmission(team, accepted), { initialProps: { team } });
  await act(async () => { await view.result.current.submit(values); });
  const id = view.result.current.attempt.id;
  const signal = apiGet.mock.calls[0][1].signal;
  view.rerender({ team: null });
  expect(signal.aborted).toBe(true);
  await act(async () => { finish(receipt(id, 'accepted')); });
  expect(accepted).not.toHaveBeenCalled();
  apiGet.mockImplementation(async (path) => receipt(idFrom(path)));
  view.rerender({ team });
  await act(async () => {});
  expect(view.result.current.attempt.request.input.prompt).toBe(values.prompt);
  apiPost.mockImplementation(async (_, request) => index(request.id));
  await act(async () => { await view.result.current.submit(values); });
  expect(apiPost.mock.calls[1][1].id).toBe(id);
  expect(accepted).toHaveBeenCalledTimes(1);
});

it('keeps polling after a missing or failed receipt and never treats either as success', async () => {
  apiPost.mockRejectedValue(new Error('connection lost'));
  apiGet.mockRejectedValueOnce(Object.assign(new Error('not found'), { status: 404 }))
    .mockRejectedValueOnce(Object.assign(new Error('unavailable'), { status: 503 }))
    .mockImplementation(async (path) => receipt(idFrom(path), 'accepted'));
  const accepted = vi.fn();
  const view = renderHook(() => useTeamSubmission(team, accepted));
  await act(async () => { await view.result.current.submit(values); });
  expect(view.result.current.confirming).toBe(true);
  expect(accepted).not.toHaveBeenCalled();
  await act(async () => { await vi.advanceTimersByTimeAsync(RECEIPT_POLL_MS); });
  expect(view.result.current.error).toContain('unavailable');
  expect(accepted).not.toHaveBeenCalled();
  await act(async () => { await vi.advanceTimersByTimeAsync(RECEIPT_POLL_MS); });
  expect(accepted).toHaveBeenCalledTimes(1);
});

it('releases a definitively rejected ID so a corrected request can be submitted', async () => {
  apiPost.mockRejectedValue(new Error('connection lost'));
  apiGet.mockImplementation(async (path) => ({ id: idFrom(path), phase: 'rejected', receipt: { status: 400, body: { error: '成员不存在' } } }));
  const accepted = vi.fn();
  const view = renderHook(() => useTeamSubmission(team, accepted));
  await act(async () => { await view.result.current.submit(values); });
  expect(view.result.current.error).toBe('成员不存在');
  expect(view.result.current.confirming).toBe(false);
  apiPost.mockImplementation(async (_, request) => index(request.id));
  await act(async () => { await view.result.current.submit(values); });
  expect(apiPost.mock.calls[0][1].id).not.toBe(apiPost.mock.calls[1][1].id);
  expect(accepted).toHaveBeenCalledTimes(1);
});

it('bounds a hung receipt read and cancels all work when unmounted', async () => {
  apiPost.mockRejectedValue(new Error('connection lost'));
  apiGet.mockImplementation(() => new Promise(() => {}));
  const view = renderHook(() => useTeamSubmission(team, vi.fn()));
  await act(async () => { await view.result.current.submit(values); });
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
  expect(view.result.current.error).toContain('等待响应超时');
  expect(apiGet.mock.calls[0][1].signal.aborted).toBe(true);
  await act(async () => { await vi.advanceTimersByTimeAsync(RECEIPT_POLL_MS); });
  expect(apiGet).toHaveBeenCalledTimes(2);
  view.unmount();
  expect(apiGet.mock.calls[1][1].signal.aborted).toBe(true);
  await vi.advanceTimersByTimeAsync(60000);
  expect(apiGet).toHaveBeenCalledTimes(2);
});

it('rejects an accepted receipt for a different ID or execution kind', () => {
  expect(() => receiptOutcome(receipt('other', 'accepted'), 'expected')).toThrow('受理结果格式无效');
  expect(() => receiptOutcome({ ...receipt('expected', 'accepted'), receipt: { body: { id: 'expected', kind: 'dag' } } }, 'expected')).toThrow('匹配');
});
