// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { MdEditDrawer } from './mdDrawer.jsx';
import { TodoDrawer } from '../todoDrawer.jsx';
import { CreateTodo } from './todoForm.jsx';

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation(async (path) => path === '/api/project/people' ? { people: [] } : { assignments: [] });
  api.apiPatch.mockResolvedValue({ ok: true });
});

it('preserves a project edit buffer when the record object refreshes', () => {
  const seed = { id: 'p1', title: '项目', detail_md: '原文' };
  const props = { open: true, initial: seed, onOk: vi.fn(), onCancel: vi.fn() };
  const view = render(<MdEditDrawer {...props} />);
  fireEvent.change(screen.getByLabelText('项目目标'), { target: { value: '未保存的正文' } });
  view.rerender(<MdEditDrawer {...props} initial={{ ...seed }} />);
  expect(screen.getByLabelText('项目目标').value).toBe('未保存的正文');
});

it('saves TODO title, description, initiative and board column together', async () => {
  const overview = { goals: [], standalone_initiatives: [], backlog: [{ id: 't1', title: '原任务', draft: '原说明', status: 'draft' }] };
  render(<TodoDrawer todoId="t1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(within(screen.getByLabelText('关键信息显示方式')).getByText('编辑'));
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '新任务' } });
  fireEvent.change(screen.getByLabelText('关键信息'), { target: { value: '新说明' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/t1', {
    title: '新任务', draft: '新说明', initiative_id: null, board_status: 'backlog', capability_id: null, tag_ids: [], owner_user_name: null, follower_user_names: [],
  }));
});

it('creates and edits unassigned TODOs with global tags', async () => {
  const overview = { goals: [], standalone_initiatives: [], tags: [{ id: 'global', name: '通用', scope_type: 'global', scope_id: '' }],
    backlog: [{ id: 't', title: '未归属任务', draft: '', tag_ids: [] }] };
  api.apiPost.mockResolvedValue({ id: 'new' });
  const create = render(<CreateTodo open overview={overview} onClose={vi.fn()} onCreated={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('标题'), { target: { value: '新任务' } });
  fireEvent.click(screen.getByRole('tab', { name: '设置', exact: true }));
  fireEvent.mouseDown(screen.getByLabelText('TODO Tag'));
  fireEvent.click(await screen.findByText('通用', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByRole('button', { name: /^创\s*建$/ }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/todos', expect.objectContaining({ initiative_id: null, tag_ids: ['global'] })));
  create.unmount();
  render(<TodoDrawer todoId="t" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '设置', exact: true }));
  fireEvent.mouseDown(screen.getByLabelText('TODO Tag'));
  fireEvent.click(await screen.findByText('通用', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/t', expect.objectContaining({ initiative_id: null, tag_ids: ['global'] })));
});

it('waits for a newly created TODO before seeding its edit buffer and preserves later edits', async () => {
  const props = { todoId: 'new', refresh: vi.fn(), onClose: vi.fn(), onNotice: vi.fn() };
  const empty = { goals: [], standalone_initiatives: [], backlog: [] };
  const view = render(<TodoDrawer {...props} overview={empty} />);
  expect(screen.queryByLabelText('TODO 标题')).toBeNull();
  const todo = { id: 'new', title: '新建任务', draft: '新建说明', board_status: 'todo' };
  const overview = { ...empty, standalone_initiatives: [{ id: 'group', title: '子项', todos: [todo] }] };
  view.rerender(<TodoDrawer {...props} overview={overview} />);
  expect(screen.getByLabelText('TODO 标题').value).toBe('新建任务');
  expect(screen.getByLabelText('关键信息').value).toBe('新建说明');
  fireEvent.click(within(screen.getByLabelText('关键信息显示方式')).getByText('编辑'));
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '未保存的标题' } });
  fireEvent.change(screen.getByLabelText('关键信息'), { target: { value: '未保存的说明' } });
  view.rerender(<TodoDrawer {...props} overview={{ ...overview }} />);
  expect(screen.getByLabelText('TODO 标题').value).toBe('未保存的标题');
  expect(screen.getByLabelText('关键信息').value).toBe('未保存的说明');
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/new', {
    title: '未保存的标题', draft: '未保存的说明', initiative_id: 'group', board_status: 'todo', capability_id: null, tag_ids: [], owner_user_name: null, follower_user_names: [],
  }));
});
