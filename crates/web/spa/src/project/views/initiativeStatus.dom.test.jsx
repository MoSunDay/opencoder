// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
import { InitiativesTab } from '../initiativesTab.jsx';
import { InitiativeDrawer } from './initiativeDrawer.jsx';
import { ProjectDrawer } from './projectDrawer.jsx';

const initiative = { id: 'i', goal_id: 'p', title: '专项甲', current_summary: '概述', todos: [
  { id: 't', title: '保留 TODO 状态', draft: '工作', initiative_id: 'i', board_status: 'done', tag_ids: [] },
] };
const overview = { goals: [{ id: 'p', title: '项目甲', status: 'active', initiatives: [initiative] }], standalone_initiatives: [], backlog: [] };
const props = { overview, refresh: vi.fn(), onNotice: vi.fn(), openInitiative: vi.fn(), openTodo: vi.fn(), onClose: vi.fn() };

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation(async () => ({ people: [] }));
  api.apiPost.mockResolvedValue({ id: 'new' });
  api.apiPatch.mockResolvedValue({ ok: true });
});

it('initiative table and create/edit forms have no state, and retain TODO progress', async () => {
  render(<InitiativesTab {...props} />);
  const table = within(screen.getByLabelText('专项表格'));
  expect(table.queryByText('状态')).toBeNull();
  expect(table.getByText('专项甲')).toBeTruthy();
  expect(table.getByText('100%')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '新建专项' }));
  expect(screen.queryByLabelText('状态')).toBeNull();
  fireEvent.change(screen.getByLabelText('标题'), { target: { value: '新方向' } });
  fireEvent.click(screen.getByRole('button', { name: /^保\s*存$/ }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledTimes(1));
  expect(api.apiPost.mock.calls[0][1]).not.toHaveProperty('status');
  fireEvent.click(screen.getByRole('button', { name: '编辑' }));
  fireEvent.change(screen.getByLabelText('标题'), { target: { value: '改名' } });
  fireEvent.click(screen.getByRole('button', { name: /^保\s*存$/ }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledTimes(1));
  expect(api.apiPatch.mock.calls[0][1]).not.toHaveProperty('status');
  expect(api.apiPatch.mock.calls[0][1]).toMatchObject({ goal_id: 'p', title: '改名' });
});

it('initiative detail retains TODO board states and edits without initiative state', async () => {
  render(<InitiativeDrawer {...props} initiativeId="i" />);
  expect(screen.getByText('保留 TODO 状态')).toBeTruthy();
  expect(screen.getAllByText(/已完成/).length).toBeGreaterThan(0);
  fireEvent.click(screen.getByRole('button', { name: '编辑专项' }));
  expect(screen.queryByLabelText('状态')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /^保\s*存$/ }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledTimes(1));
  expect(api.apiPatch.mock.calls[0][1]).not.toHaveProperty('status');
});

it('project detail creates initiatives without state', async () => {
  render(<ProjectDrawer {...props} projectId="p" />);
  expect(within(screen.getByLabelText('项目内专项表格')).queryByText('状态')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '新建专项' }));
  fireEvent.change(screen.getByLabelText('标题'), { target: { value: '项目内专项' } });
  fireEvent.click(screen.getByRole('button', { name: /^保\s*存$/ }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledTimes(1));
  expect(api.apiPost.mock.calls[0][1]).not.toHaveProperty('status');
  expect(api.apiPost.mock.calls[0][1].goal_id).toBe('p');
});
