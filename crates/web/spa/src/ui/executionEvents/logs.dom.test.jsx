// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
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
  expect(get).toHaveBeenLastCalledWith('/api/executions/run/events-page?after=1');
  expect(screen.getByRole('log').textContent).toContain('next log');
  expect(screen.getByRole('button', { name: '下一页' }).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '返回实时日志' }));
  expect(screen.getByRole('log').textContent).toContain('live log');
});
