// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
import { apiGet, apiPost } from '../../api.js';
import { setIdentity } from '../../store.js';
import { TodoComments } from './panel.jsx';
import { useComments } from './useComments.js';

const comment = (seq, body = `note-${seq}`) => ({ seq, body_md: body, author_kind: 'user', author_name: 'alice', author_id: 'alice', submitted_by: 'alice', created_at: 1 });
const props = () => ({ todoId: 't', active: true, onDraftChange: vi.fn(), onBusyChange: vi.fn(), openExecution: vi.fn(), refresh: vi.fn() });
beforeEach(() => {
  vi.resetAllMocks(); setIdentity({ name: 'alice', role: 'editor' });
  apiGet.mockResolvedValue({ comments: [], has_more: false });
});
afterEach(() => vi.useRealTimers());

it('keeps failed Markdown comments and retries the same request without saving the task', async () => {
  apiPost.mockRejectedValueOnce(new Error('评论保存失败')).mockResolvedValueOnce(comment(1, '**补充进展**'));
  const p = props(); render(<TodoComments {...p} />);
  await screen.findByText('暂无评论');
  fireEvent.change(screen.getByLabelText('评论正文'), { target: { value: '**补充进展**' } });
  fireEvent.click(screen.getByRole('button', { name: /发表评论$/ }));
  expect(await screen.findByText('评论保存失败')).toBeTruthy();
  expect(screen.getByLabelText('评论正文').value).toBe('**补充进展**');
  const input = apiPost.mock.calls[0][1];
  expect(input).not.toHaveProperty('author_id'); expect(input).not.toHaveProperty('capability_id');
  fireEvent.click(screen.getByRole('button', { name: /发表评论$/ }));
  await waitFor(() => expect(p.refresh).toHaveBeenCalledOnce());
  expect(apiPost.mock.calls[1][1]).toEqual(input);
  expect(screen.getByText('补充进展').tagName).toBe('STRONG');
  expect(screen.getByLabelText('评论正文').value).toBe('');
});

it('shows capability and submitting user separately and opens the native source', async () => {
  apiGet.mockResolvedValue({ comments: [{ ...comment(1), author_kind: 'capability', author_id: 'builtin-operator', author_name: 'act', capability_kind: 'operator', execution_id: 'operator-source' }], has_more: false });
  const p = props(); render(<TodoComments {...p} />);
  expect(await screen.findByText('builtin-operator · 提交用户：alice')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '查看执行来源' }));
  expect(p.openExecution).toHaveBeenCalledWith('operator-source');
});

it('aborts hidden reads and never replaces comments with a late response', async () => {
  let resolve;
  let signal;
  apiGet.mockImplementationOnce((_, options) => { signal = options.signal; return new Promise((done) => { resolve = done; }); });
  const view = render(<TodoComments {...props()} />);
  view.rerender(<TodoComments {...props()} active={false} />);
  expect(signal.aborted).toBe(true);
  apiGet.mockResolvedValue({ comments: [comment(2, '新内容')], has_more: false });
  view.rerender(<TodoComments {...props()} />);
  expect(await screen.findByText('新内容')).toBeTruthy();
  await act(async () => resolve({ comments: [comment(1, '旧响应')], has_more: false }));
  expect(screen.queryByText('旧响应')).toBeNull();
});

it('loads every incremental page and pauses polling while hidden', async () => {
  vi.useFakeTimers();
  apiGet.mockImplementation(async (path) => path.includes('after_seq=3') ? { comments: [], has_more: false }
    : path.includes('after_seq=2') ? { comments: [comment(3)], has_more: false }
    : path.includes('after_seq=1') ? { comments: [comment(2)], has_more: true }
    : { comments: [comment(1)], has_more: false });
  const view = renderHook(({ active }) => useComments('t', active), { initialProps: { active: true } });
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(view.result.current.rows.map((row) => row.seq)).toEqual([1, 2, 3]);
  view.rerender({ active: false });
  const count = apiGet.mock.calls.length;
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
  expect(apiGet).toHaveBeenCalledTimes(count);
});
