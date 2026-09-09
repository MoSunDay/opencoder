// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionDetail: () => null }));
import { MdEditModal } from './mdModal.jsx';
import { TodoDrawer } from '../todoDrawer.jsx';
import { ProjectPanel } from '../project.jsx';

const button = (label) => [...document.querySelectorAll('button')].find((b) => b.textContent.replace(/\s/g, '') === label);
const seed = { id: 'g1', title: '项目', sort: 2, detail_md: '服务器正文' };
const change = (label, value) => fireEvent.change(screen.getByLabelText(label), { target: { value } });
const defer = () => { let resolve; let reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const todo = { id: 't1', title: '任务', draft: '原始草稿', agent: 'act', status: 'draft', updated_at: 1 };
const overview = (record = todo) => ({ goals: [], standalone_milestones: [], backlog: [record] });

beforeEach(() => {
  vi.useRealTimers();
  Object.values(api).forEach((fn) => fn.mockReset());
  api.apiGet.mockResolvedValue({ runs: [] });
  api.apiPatch.mockResolvedValue({ ok: true });
});

describe('editing buffers', () => {
  it('keeps all fields and preview mode when the same record receives a new object', () => {
    const props = { open: true, initial: seed, onOk: vi.fn(), onCancel: vi.fn() };
    const view = render(<MdEditModal {...props} />);
    change('detail_md', '# 未保存的正文');
    fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '未保存的标题' } });
    fireEvent.click(screen.getByText('预览'));
    view.rerender(<MdEditModal {...props} initial={{ ...seed }} />);
    expect(screen.getByLabelText('detail_preview').textContent.trim()).toBe('未保存的正文');
    expect(screen.getByPlaceholderText('一句话标题').value).toBe('未保存的标题');
    fireEvent.click(screen.getByText('编辑'));
    expect(screen.getByLabelText('detail_md').value).toBe('# 未保存的正文');
  });

  it('saves from preview and retains every field after a failed save', async () => {
    const save = vi.fn().mockResolvedValue(false);
    render(<MdEditModal open initial={seed} onOk={save} onCancel={vi.fn()} />);
    change('detail_md', '新的 **正文**');
    fireEvent.click(screen.getByText('预览'));
    fireEvent.click(button('保存'));
    await waitFor(() => expect(save).toHaveBeenCalledWith(expect.objectContaining({ detail_md: '新的 **正文**', title: '项目', sort: 2 })));
    expect(screen.getByLabelText('detail_preview').textContent.trim()).toBe('新的 正文');
    fireEvent.click(screen.getByText('编辑'));
    expect(screen.getByLabelText('detail_md').value).toBe('新的 **正文**');
  });

  it('initializes a fresh session when cancelled and reopened or changing record', () => {
    const props = { open: true, initial: seed, onOk: vi.fn(), onCancel: vi.fn() };
    const view = render(<MdEditModal {...props} />);
    change('detail_md', '临时草稿');
    view.rerender(<MdEditModal {...props} open={false} />);
    view.rerender(<MdEditModal {...props} />);
    expect(screen.getByLabelText('detail_md').value).toBe(seed.detail_md);
    view.rerender(<MdEditModal {...props} initial={{ id: 'g2', title: '另一个项目', detail_md: '第二份正文' }} />);
    expect(screen.getByLabelText('detail_md').value).toBe('第二份正文');
  });

  it.each([['draft', 8000], ['running', 3000]])('survives actual %s overview polling', async (status, interval) => {
    const intervals = vi.spyOn(globalThis, 'setInterval');
    let calls = 0;
    api.apiGet.mockImplementation(async (path) => path === '/api/project/overview'
      ? { goals: [{ ...seed, updated_at: ++calls, milestones: [] }], backlog: [{ ...todo, status }] } : { runs: [] });
    render(<ProjectPanel onNotice={vi.fn()} />);
    fireEvent.click(screen.getByRole('tab', { name: '项目目标' }));
    await screen.findByText('项目');
    fireEvent.click(button('编辑'));
    change('detail_md', '轮询中编辑');
    const poll = intervals.mock.calls.findLast(([, ms]) => ms === interval)?.[0];
    expect(poll).toBeTypeOf('function');
    const before = calls;
    await act(async () => { await poll(); });
    expect(calls).toBeGreaterThan(before);
    expect(screen.getByLabelText('detail_md').value).toBe('轮询中编辑');
    intervals.mockRestore();
  });

  it('keeps TODO draft during execution updates and after saving before overview catches up', async () => {
    const refresh = vi.fn().mockResolvedValue(undefined);
    const props = { todoId: 't1', overview: overview(), refresh, onNotice: vi.fn(), onClose: vi.fn() };
    const view = render(<TodoDrawer {...props} />);
    change('todo-draft', '用户未保存的草稿');
    view.rerender(<TodoDrawer {...props} overview={overview({ ...todo, status: 'running', updated_at: 2 })} />);
    expect(screen.getByLabelText('todo-draft').value).toBe('用户未保存的草稿');
    fireEvent.click(button('保存草稿'));
    await waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(screen.getByLabelText('todo-draft').value).toBe('用户未保存的草稿');
    expect(button('保存草稿').disabled).toBe(true);
  });

  it('failed TODO saves keep the draft; an old save cannot overwrite a different TODO', async () => {
    const failed = defer(); api.apiPatch.mockReturnValueOnce(failed.promise);
    const props = { todoId: 't1', overview: overview(), refresh: vi.fn(), onNotice: vi.fn(), onClose: vi.fn() };
    const view = render(<TodoDrawer {...props} />);
    change('todo-draft', '不能丢'); fireEvent.click(button('保存草稿'));
    await act(async () => failed.reject(new Error('save failed')));
    expect(screen.getByLabelText('todo-draft').value).toBe('不能丢');
    const late = defer(); api.apiPatch.mockReturnValueOnce(late.promise);
    fireEvent.click(button('保存草稿'));
    view.rerender(<TodoDrawer {...props} todoId="t2" overview={overview({ ...todo, id: 't2', draft: '任务二' })} />);
    change('todo-draft', '任务二编辑');
    await act(async () => late.resolve({ ok: true }));
    expect(screen.getByLabelText('todo-draft').value).toBe('任务二编辑');
  });
});
