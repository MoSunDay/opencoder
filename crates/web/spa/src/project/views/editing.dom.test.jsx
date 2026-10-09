// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { MdEditDrawer } from './mdDrawer.jsx';
import { TodoDrawer } from '../todoDrawer.jsx';

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockResolvedValue({ assignments: [] });
  api.apiPatch.mockResolvedValue({ ok: true });
});

it('preserves a project edit buffer when the record object refreshes', () => {
  const seed = { id: 'p1', title: '项目', detail_md: '原文' };
  const props = { open: true, initial: seed, onOk: vi.fn(), onCancel: vi.fn() };
  const view = render(<MdEditDrawer {...props} />);
  fireEvent.change(screen.getByLabelText('detail_md'), { target: { value: '未保存的正文' } });
  view.rerender(<MdEditDrawer {...props} initial={{ ...seed }} />);
  expect(screen.getByLabelText('detail_md').value).toBe('未保存的正文');
});

it('saves TODO title, description, initiative and board column together', async () => {
  const overview = { goals: [], standalone_initiatives: [], backlog: [{ id: 't1', title: '原任务', draft: '原说明', status: 'draft' }] };
  render(<TodoDrawer todoId="t1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '新任务' } });
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '新说明' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/t1', {
    title: '新任务', draft: '新说明', initiative_id: null, board_status: 'backlog', capability_id: null, tag_ids: [],
  }));
});

it('waits for a newly created TODO before seeding its edit buffer and preserves later edits', async () => {
  const props = { todoId: 'new', refresh: vi.fn(), onClose: vi.fn(), onNotice: vi.fn() };
  const empty = { goals: [], standalone_initiatives: [], backlog: [] };
  const view = render(<TodoDrawer {...props} overview={empty} />);
  expect(screen.queryByLabelText('TODO 标题')).toBeNull();
  const todo = { id: 'new', title: '新建任务', draft: '新建说明', board_status: 'todo' };
  const overview = { ...empty, standalone_initiatives: [{ id: 'group', title: '专项', todos: [todo] }] };
  view.rerender(<TodoDrawer {...props} overview={overview} />);
  expect(screen.getByLabelText('TODO 标题').value).toBe('新建任务');
  expect(screen.getByLabelText('任务说明').value).toBe('新建说明');
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '未保存的标题' } });
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '未保存的说明' } });
  view.rerender(<TodoDrawer {...props} overview={{ ...overview }} />);
  expect(screen.getByLabelText('TODO 标题').value).toBe('未保存的标题');
  expect(screen.getByLabelText('任务说明').value).toBe('未保存的说明');
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/new', {
    title: '未保存的标题', draft: '未保存的说明', initiative_id: 'group', board_status: 'todo', capability_id: null, tag_ids: [],
  }));
});
