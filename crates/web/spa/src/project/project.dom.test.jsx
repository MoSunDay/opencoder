// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
vi.mock('../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { ProjectPanel } from './project.jsx';

const snapshot = {
  goals: [{ id: 'p1', title: '项目甲', status: 'active', sort: 0, milestones: [{
    id: 'm1', title: '里程碑甲', status: 'planned', sort: 0, todos: [],
  }], initiatives: [{
    id: 's1', title: '专项甲', status: 'planned', sort: 0, todos: [
      { id: 't1', title: '专项任务', draft: '说明', status: 'draft', milestone_id: 's1', latest_assignment: { execution_id: 'agent-1', has_result: true } },
    ],
  }] }],
  standalone_initiatives: [{ id: 's2', title: '独立专项', status: 'planned', sort: 0, todos: [] }],
  backlog: [{ id: 't2', title: '未分组任务', draft: '', status: 'draft', milestone_id: null }],
};

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/project/overview' ? snapshot : { assignments: [] }));
  api.apiPost.mockResolvedValue({ id: 'created' });
  api.apiPatch.mockResolvedValue({ ok: true });
  api.apiDel.mockResolvedValue({ deleted: true });
});

it('shows distinct milestones and initiatives with grouped and ungrouped TODOs', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  expect(await screen.findByText('项目甲')).toBeTruthy();
  fireEvent.click(screen.getByRole('tab', { name: '里程碑' }));
  expect(await screen.findByText('里程碑甲')).toBeTruthy();
  fireEvent.click(screen.getByRole('tab', { name: '专项' }));
  expect(await screen.findByText('专项甲')).toBeTruthy();
  expect(screen.getByText('独立专项')).toBeTruthy();
  fireEvent.click(screen.getByRole('tab', { name: 'TODO' }));
  expect(await screen.findByText('专项任务')).toBeTruthy();
  expect(screen.getByText('结论已回写')).toBeTruthy();
  expect(screen.getByText('未分组任务')).toBeTruthy();
  expect(screen.queryByText('生成Plan')).toBeNull();
});

it('opens the right-side TODO drawer and reads execution IDs separately', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: 'TODO' }));
  fireEvent.click(await screen.findByText('未分组任务'));
  expect(await screen.findByText('TODO · 未分组任务')).toBeTruthy();
  await waitFor(() => expect(api.apiGet).toHaveBeenCalledWith('/api/project/todos/t2/executions'));
});
