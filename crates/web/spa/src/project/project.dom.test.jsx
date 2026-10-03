// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn(), apiPut: vi.fn() }));
vi.mock('../api.js', () => api);
vi.mock('../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { ProjectPanel } from './project.jsx';
const snapshot = {
  tags: [{ id: 'tag1', scope_type: 'project', scope_id: 'p1', name: '前端' }],
  goals: [{ id: 'p1', title: '项目甲', status: 'active', updated_at: 1000, initiatives: [{ id: 's1', title: '专项甲', goal_id: 'p1', status: 'planned', todos: [
    { id: 't1', title: '专项任务', draft: '说明', board_status: 'todo', initiative_id: 's1', tag_ids: ['tag1'] },
    { id: 't2', title: '完成任务', board_status: 'done', initiative_id: 's1', tag_ids: [] },
  ] }] }], standalone_initiatives: [{ id: 's2', title: '独立专项', status: 'planned', todos: [] }],
  backlog: [{ id: 't3', title: '未归属任务', draft: '', board_status: 'backlog', tag_ids: [] }],
};
beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/project/overview' ? snapshot : { assignments: [] }));
  api.apiPost.mockResolvedValue({ id: 'created' }); api.apiPatch.mockResolvedValue({ ok: true }); api.apiDel.mockResolvedValue({ deleted: true });
});
it('has exactly three tabs and a global TODO table including unassigned work', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  expect(await screen.findByText('项目甲')).toBeTruthy();
  expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['项目', '专项', 'TODO']);
  fireEvent.click(screen.getByRole('tab', { name: 'TODO' }));
  const table = await screen.findByLabelText('TODO 表格');
  expect(within(table).getByText('专项任务')).toBeTruthy();
  expect(within(table).getByText('未归属任务')).toBeTruthy();
  expect(within(table).getByText('前端')).toBeTruthy();
  expect(screen.queryByRole('button', { name: '拖动 专项任务' })).toBeNull();
});
it('opens project progress, then the initiative board in a right-side drawer', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  fireEvent.click(await screen.findByRole('button', { name: '项目甲' }));
  const project = (await screen.findByText('项目 · 项目甲', { exact: true })).closest('[role=dialog]');
  expect(within(project).getAllByText('50%').length).toBeGreaterThan(0);
  expect(within(project).getAllByText('1/2').length).toBeGreaterThan(0);
  fireEvent.click(within(project).getByText('专项甲', { exact: true }).closest('button'));
  const initiative = (await screen.findByText('专项 · 专项甲', { exact: true })).closest('[role=dialog]');
  expect(within(initiative).getByLabelText('拖动 专项任务').disabled).toBe(false);
  fireEvent.change(within(initiative).getByLabelText('搜索专项 TODO'), { target: { value: '专项任务' } });
  expect(within(initiative).getByLabelText('拖动 专项任务').disabled).toBe(false);
  expect(within(initiative).queryByText('完成任务')).toBeNull();
  fireEvent.click(within(initiative).getByText('专项任务', { exact: true }));
  expect((await screen.findByText('TODO · 专项任务', { exact: true })).closest('[role=dialog]')).toBeTruthy();
  await waitFor(() => expect(api.apiGet).toHaveBeenCalledWith('/api/project/todos/t1/executions', { signal: expect.any(AbortSignal) }));
}, 60000);
it('filters a table by its title column and preserves the filter after a refresh', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: 'TODO' }));
  const table = await screen.findByLabelText('TODO 表格');
  fireEvent.click(within(table).getByLabelText('筛选TODO'));
  const search = await screen.findByLabelText('搜索TODO');
  fireEvent.change(search, { target: { value: '未归属' } }); fireEvent.keyDown(search, { key: 'Enter', code: 'Enter' });
  await waitFor(() => expect(within(table).queryByText('专项任务')).toBeNull());
  expect(within(table).getByText('未归属任务')).toBeTruthy();
  fireEvent.click(within(table).getByText('未归属任务', { exact: true }));
  const detail = (await screen.findByText('TODO · 未归属任务', { exact: true })).closest('[role=dialog]');
  const before = api.apiGet.mock.calls.filter(([path]) => path === '/api/project/overview').length;
  fireEvent.click(within(detail).getByText('保存 TODO', { exact: true }));
  await waitFor(() => expect(api.apiGet.mock.calls.filter(([path]) => path === '/api/project/overview').length).toBeGreaterThan(before));
  fireEvent.click(detail.querySelector('.ant-drawer-close'));
  expect(within(table).queryByText('专项任务')).toBeNull();
  expect(within(table).getByText('未归属任务')).toBeTruthy();
  fireEvent.click(within(table).getByRole('button', { name: '清除全部列筛选' }));
  expect(within(table).getByText('专项任务')).toBeTruthy();
});
