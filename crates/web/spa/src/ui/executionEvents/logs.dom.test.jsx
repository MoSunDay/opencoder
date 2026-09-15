// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const open = vi.hoisted(() => vi.fn());
const get = vi.hoisted(() => vi.fn());
vi.mock('../../api.js', () => ({ apiGet: get }));
vi.mock('../../sse.js', () => ({ openStream: open }));
import { useExecutionEvents } from './useExecutionEvents.js';
import { ExecutionLogs } from './executionLogs.jsx';
const frame = (seq, step, text) => ({ seq, event: 'step_log', data: { step, payload: { event: 'stdout', data: { text } } } });
beforeEach(() => { open.mockReset().mockImplementation(() => ({ abort: vi.fn() })); get.mockReset(); });
it('resumes from the last delivered cursor, retains connection failure and resets for a new execution', () => {
  const { result, rerender, unmount } = renderHook(({ id }) => useExecutionEvents({ id }), { initialProps: { id: 'a' } });
  act(() => open.mock.calls[0][0].onFrame(frame(4, 'build', 'one')));
  act(() => { open.mock.calls[0][0].onStatus('failed'); open.mock.calls[0][0].onStatus('closed'); });
  expect(result.current.connection).toBe('failed');
  act(() => result.current.retry());
  expect(open.mock.calls[1][0]).toMatchObject({ after: 4, executionHistory: true, requireEnd: true });
  act(() => { open.mock.calls[1][0].onFrame(frame(4, 'build', 'duplicate')); open.mock.calls[1][0].onFrame(frame(5, 'build', 'two')); });
  expect(result.current.frames.map((f) => f.seq)).toEqual([4, 5]);
  rerender({ id: 'b' });
  expect(result.current.frames).toEqual([]);
  expect(open.mock.calls.at(-1)[0].after).toBe(0);
  const last = open.mock.results.at(-1).value;
  unmount();
  expect(last.abort).toHaveBeenCalledOnce();
});
it('shows incremental output, filters by step and reports a broken connection', () => {
  const props = { id: 'a', steps: ['build', 'test'], frames: [frame(1, 'build', 'compiling'), frame(2, 'test', 'testing')] };
  const { rerender } = render(<ExecutionLogs {...props} />);
  expect(screen.getByRole('log').textContent).toContain('compiling');
  rerender(<ExecutionLogs {...props} step="build" frames={[...props.frames, frame(3, 'build', ' finished')]} connection="failed" />);
  expect(screen.getByRole('log').textContent).toContain('compiling finished');
  expect(screen.getByRole('log').textContent).not.toContain('testing');
  expect(screen.getByText('日志连接失败，请重新连接')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('搜索日志'), { target: { value: 'absent' } });
  expect(screen.getByText('没有匹配的日志')).toBeTruthy();
  fireEvent.click(screen.getByRole('switch', { name: '自动滚动' }));
  expect(screen.getByRole('switch', { name: '自动滚动' }).getAttribute('aria-checked')).toBe('false');
});

it('pages older events without interrupting live output and returns to the latest logs', async () => {
  get.mockResolvedValueOnce({ events: [{ seq: 1, kind: 'stdout', data: { text: 'old log' } }], more: true, finished: true })
    .mockResolvedValueOnce({ events: [{ seq: 2, kind: 'stderr', data: { text: 'next log' } }], more: false, finished: true });
  render(<ExecutionLogs id="run" frames={[frame(100, 'a', 'live log')]} trimmed />);
  await act(async () => fireEvent.click(screen.getByRole('button', { name: '从头查看历史' })));
  expect(screen.getByRole('log').textContent).toContain('old log');
  await act(async () => fireEvent.click(screen.getByRole('button', { name: '下一页' })));
  expect(get).toHaveBeenLastCalledWith('/api/executions/run/events-page?after=1', expect.objectContaining({ signal: expect.any(AbortSignal) }));
  expect(screen.getByRole('log').textContent).toContain('next log');
  expect(screen.getByRole('button', { name: '下一页' }).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '返回实时日志' }));
  expect(screen.getByRole('log').textContent).toContain('live log');
});

it('loads historical pages atomically before subscribing after their last cursor', async () => {
  let finish;
  get.mockResolvedValueOnce({ head_seq: 8, events: [{ seq: 3, kind: 'stdout', data: { text: 'old' } }], more: true, finished: false })
    .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const { result, unmount } = renderHook(() => useExecutionEvents({ id: 'history', hydrate: true }));
  await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
  expect(result.current.frames).toEqual([]);
  expect(open).not.toHaveBeenCalled();
  await act(async () => finish({ head_seq: 8, events: [{ seq: 8, kind: 'stdout', data: { text: 'new' } }], more: false, finished: false }));
  expect(result.current.frames.map((f) => f.seq)).toEqual([3, 8]);
  expect(open.mock.calls[0][0].after).toBe(8);
  const handle = open.mock.calls[0][0];
  act(() => handle.onFrame(frame(9, 'a', 'live')));
  expect(result.current.cursor).toBe(9);
  unmount();
  expect(open.mock.results[0].value.abort).toHaveBeenCalledOnce();
});

it('aborts initial history loading on close and rejects a stalled page', async () => {
  let finish;
  get.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const first = renderHook(() => useExecutionEvents({ id: 'closed', hydrate: true }));
  const signal = get.mock.calls[0][1].signal;
  first.unmount();
  expect(signal.aborted).toBe(true);
  await act(async () => finish({ head_seq: 0, events: [], more: false, finished: false }));
  expect(open).not.toHaveBeenCalled();
  get.mockResolvedValue({ head_seq: 5, events: [], more: true, finished: false });
  const next = renderHook(() => useExecutionEvents({ id: 'stalled', hydrate: true }));
  await waitFor(() => expect(next.result.current.connection).toBe('failed'));
  expect(next.result.current.error).toContain('分页');
  expect(open).not.toHaveBeenCalled();
});

it('scrolls new output only while automatic scrolling is enabled', () => {
  const props = { id: 'a', frames: [frame(1, 'build', 'one')] };
  const { rerender } = render(<ExecutionLogs {...props} />);
  const element = screen.getByRole('log');
  Object.defineProperty(element, 'scrollHeight', { value: 600 });
  rerender(<ExecutionLogs {...props} frames={[...props.frames, frame(2, 'build', 'two')]} />);
  expect(element.scrollTop).toBe(600);
  fireEvent.click(screen.getByRole('switch', { name: '自动滚动' }));
  element.scrollTop = 100;
  rerender(<ExecutionLogs {...props} frames={[...props.frames, frame(3, 'build', 'three')]} />);
  expect(element.scrollTop).toBe(100);
});
