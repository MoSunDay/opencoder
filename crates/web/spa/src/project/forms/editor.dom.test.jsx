// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { apiGet, apiPatch, apiPost } from '../../api.js';
import { setIdentity } from '../../store.js';
import { TodoDrawer } from '../todoDrawer.jsx';
import { CreateTodo } from '../views/todoForm.jsx';

const todo = { id: 't', title: '任务标题', draft: '原说明', board_status: 'todo', tag_ids: [] };
const overview = { goals: [], standalone_initiatives: [], backlog: [todo], tags: [] };
const props = () => ({ todoId: 't', overview, onNotice: vi.fn(), refresh: vi.fn(), onClose: vi.fn() });
beforeEach(() => {
  vi.resetAllMocks(); setIdentity({ name: 'editor', role: 'editor' });
  apiGet.mockImplementation(async (path) => path.endsWith('/people') ? { people: [{ name: 'alice' }] } : path.endsWith('/library') ? { capabilities: [] } : { assignments: [] });
  apiPatch.mockResolvedValue({ ok: true });
});

it('gives every editor control a visible associated label inside a named form section', async () => {
  render(<TodoDrawer {...props()} />);
  for (const [group, labels] of [
    ['任务内容', ['TODO 标题', '任务说明']], ['归属与状态', ['所属专项', 'TODO 看板列', 'TODO Tag']],
    ['负责人和关注人', ['负责人', '关注人']], ['执行设置', ['执行能力']], ['指派记录', ['已有执行 ID']],
  ]) {
    const section = screen.getByRole('region', { name: group });
    for (const label of labels) {
      const control = within(section).getByLabelText(label);
      expect(control.id).toBeTruthy();
      expect([...section.querySelectorAll('label')].some((node) => node.htmlFor === control.id && node.textContent)).toBe(true);
    }
  }
  await waitFor(() => expect(screen.getByLabelText('负责人').disabled).toBe(false));
  expect(screen.getByRole('button', { name: '保存 TODO' }).closest('.ant-drawer-footer')).toBeTruthy();
});

it('retains failed edits and prevents field changes while a save is pending', async () => {
  let reject;
  apiPatch.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  const p = props(); const view = render(<TodoDrawer {...p} />);
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '   ' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  expect(await screen.findByText('TODO 标题不能为空')).toBeTruthy();
  expect(document.activeElement).toBe(screen.getByLabelText('TODO 标题'));
  expect(apiPatch).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '保留我的修改' } });
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '尚未保存的完整说明' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(screen.getByLabelText('TODO 标题').disabled).toBe(true));
  view.rerender(<TodoDrawer {...p} overview={{ ...overview, backlog: [{ ...todo, title: '远端刷新' }] }} />);
  await act(async () => reject(new Error('保存失败，请重试')));
  expect(await screen.findByText('保存失败，请重试')).toBeTruthy();
  expect(screen.getByText('保存失败，请重试').closest('.ant-drawer-footer')).toBeTruthy();
  expect(screen.getByLabelText('TODO 标题').value).toBe('保留我的修改');
  expect(screen.getByLabelText('任务说明').value).toBe('尚未保存的完整说明');
  expect(screen.getByLabelText('TODO 标题').disabled).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: /保存 TODO/ }));
  await waitFor(() => expect(p.refresh).toHaveBeenCalledOnce());
  expect(apiPatch).toHaveBeenLastCalledWith('/api/project/todos/t', expect.objectContaining({ title: '保留我的修改', draft: '尚未保存的完整说明' }));
});

it('keeps create errors and drafts in place and exposes creation in the drawer footer', async () => {
  apiPost.mockRejectedValue(new Error('创建暂时失败'));
  const onNotice = vi.fn();
  render(<CreateTodo open overview={overview} onClose={vi.fn()} onCreated={vi.fn()} onNotice={onNotice} />);
  expect(screen.getByRole('region', { name: '归属与状态' })).toBeTruthy();
  fireEvent.change(screen.getByLabelText('标题'), { target: { value: '新任务' } });
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '需要保留的说明' } });
  const create = screen.getByRole('button', { name: /^创\s*建$/ });
  expect(create.closest('.ant-drawer-footer')).toBeTruthy();
  fireEvent.click(create);
  await waitFor(() => expect(onNotice).toHaveBeenCalledWith(expect.objectContaining({ text: '创建暂时失败' })));
  expect(screen.getByText('创建暂时失败')).toBeTruthy();
  expect(screen.getByText('创建暂时失败').closest('.ant-drawer-footer')).toBeTruthy();
  expect(screen.getByLabelText('标题').value).toBe('新任务');
  expect(screen.getByLabelText('任务说明').value).toBe('需要保留的说明');
});

it('keeps read-only fields and both footer actions disabled', async () => {
  setIdentity({ name: 'viewer', role: 'viewer' });
  render(<TodoDrawer {...props()} />);
  for (const label of ['TODO 标题', '任务说明', '所属专项', 'TODO 看板列', 'TODO Tag', '负责人', '关注人', '执行能力', '已有执行 ID']) {
    expect(screen.getByLabelText(label).disabled).toBe(true);
  }
  expect(screen.getByRole('button', { name: '保存 TODO' }).disabled).toBe(true);
  expect(screen.getByRole('button', { name: '指派所选能力' }).disabled).toBe(true);
});
