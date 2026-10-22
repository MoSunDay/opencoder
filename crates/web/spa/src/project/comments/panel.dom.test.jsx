// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
import { apiGet, apiPost, apiPatch, apiDel } from '../../api.js';
import { setIdentity } from '../../store.js';
import { TodoComments } from './panel.jsx';
import { useComments } from './useComments.js';

const comment = (seq, body = `note-${seq}`) => ({ seq, revision: 1, body_md: body, author_kind: 'user', author_name: 'alice', author_id: 'alice', submitted_by: 'alice', created_at: 1 });
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

it('refreshes loaded history after remote edits and deletion and pauses while hidden', async () => {
  vi.useFakeTimers();
  let current = [comment(1), comment(2)];
  apiGet.mockImplementation(async () => ({ comments: current, has_more: false }));
  const view = renderHook(({ active }) => useComments('t', active), { initialProps: { active: true } });
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  current = [{ ...comment(1, 'updated remotely'), revision: 2 }, comment(3)];
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(view.result.current.rows.map((row) => row.seq)).toEqual([1, 3]);
  expect(view.result.current.rows[0].body_md).toBe('updated remotely');
  view.rerender({ active: false });
  const count = apiGet.mock.calls.length;
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
  expect(apiGet).toHaveBeenCalledTimes(count);
});

it('retains an edit on failure and saves with the original revision', async () => {
  apiGet.mockResolvedValue({ comments: [comment(1)], has_more: false });
  apiPatch.mockRejectedValueOnce(new Error('评论已变化')).mockResolvedValueOnce({ ...comment(1, '**merged**'), revision: 2 });
  const p = props(); render(<TodoComments {...p} />);
  fireEvent.click(await screen.findByRole('button', { name: '编辑评论' }));
  fireEvent.change(screen.getByLabelText('编辑评论正文'), { target: { value: '**merged**' } });
  await waitFor(() => expect(screen.getByRole('button', { name: /保存评论$/ }).classList.contains('ant-btn-loading')).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: /保存评论$/ }));
  expect(await screen.findByText('评论已变化')).toBeTruthy();
  expect(screen.getByLabelText('编辑评论正文').value).toBe('**merged**');
  expect(p.onDraftChange).toHaveBeenLastCalledWith(true);
  await waitFor(() => expect(screen.getByRole('button', { name: /保存评论$/ }).classList.contains('ant-btn-loading')).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: /保存评论$/ }));
  expect(await screen.findByText('merged')).toBeTruthy();
  expect(apiPatch).toHaveBeenLastCalledWith('/api/project/todos/t/comments/1', { body_md: '**merged**', expected_revision: 1 });
  expect(screen.getByText('已编辑')).toBeTruthy();
  expect(apiPost).not.toHaveBeenCalled();
});

it('requires deletion confirmation and preserves the comment on failure', async () => {
  apiGet.mockResolvedValue({ comments: [comment(1)], has_more: false });
  apiDel.mockRejectedValueOnce(new Error('删除失败')).mockResolvedValueOnce({ ok: true });
  render(<TodoComments {...props()} />);
  fireEvent.click(await screen.findByRole('button', { name: '删除评论' }));
  expect(apiDel).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByRole('button', { name: /删\s*除$/ }));
  expect(await screen.findByText('删除失败')).toBeTruthy();
  expect(screen.getByText('note-1')).toBeTruthy();
  await waitFor(() => expect(screen.getByRole('button', { name: '删除评论' }).disabled).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: '删除评论' }));
  fireEvent.click(await screen.findByRole('button', { name: /删\s*除$/ }));
  await waitFor(() => expect(screen.queryByText('note-1')).toBeNull());
  expect(apiDel).toHaveBeenLastCalledWith('/api/project/todos/t/comments/1?expected_revision=1');
});

it('restricts mutation controls to the submitting editor or an administrator', async () => {
  apiGet.mockResolvedValue({ comments: [comment(1)], has_more: false });
  setIdentity({ name: 'bob', role: 'editor' });
  render(<TodoComments {...props()} />);
  await screen.findByText('note-1');
  expect(screen.queryByRole('button', { name: '编辑评论' })).toBeNull();
  expect(screen.queryByRole('button', { name: '删除评论' })).toBeNull();
  act(() => setIdentity({ name: 'admin', role: 'admin' }));
  expect(await screen.findByRole('button', { name: '编辑评论' })).toBeTruthy();
});

it('discards a polling response that predates a local mutation', async () => {
  vi.useFakeTimers();
  let resolve;
  apiGet.mockResolvedValueOnce({ comments: [comment(1)], has_more: false })
    .mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
  const view = renderHook(() => useComments('t', true));
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  act(() => view.result.current.remove(1));
  await act(async () => resolve({ comments: [comment(1)], has_more: false }));
  expect(view.result.current.rows).toEqual([]);
});
