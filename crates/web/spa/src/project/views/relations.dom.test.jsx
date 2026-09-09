// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionDetail: () => null }));
import { ProjectPanel } from '../project.jsx';
import { RelationSelect } from './relationSelect.jsx';
import { flattenTodos, flattenMilestones, milestoneOptions } from '../model/relations.js';
import { milestoneCards, ownerLabel } from '../progressPanel.jsx';

const snapshot = { goals: [], standalone_milestones: [{ id: 'm1', goal_id: null, title: '独立专项', status: 'planned', todos: [
  { id: 't1', title: '专项任务', draft: 'd', status: 'draft', milestone_id: 'm1' },
] }], backlog: [{ id: 't2', title: '独立任务', draft: 'd', status: 'draft' }] };
const button = (label) => [...document.querySelectorAll('button')].find((b) => b.textContent.replace(/\s/g, '') === label);
beforeEach(() => { Object.values(api).forEach((fn) => fn.mockReset()); api.apiGet.mockResolvedValue(snapshot); api.apiPost.mockResolvedValue({ id: 'm2' }); api.apiPatch.mockResolvedValue({ ok: true }); });

it('projects, selectors and rollups include independent work exactly once', () => {
  expect(flattenTodos(snapshot).map((t) => t.id)).toEqual(['t1', 't2']);
  expect(flattenMilestones(snapshot)).toHaveLength(1);
  expect(milestoneCards(snapshot)[0]).toMatchObject({ total: 1, goal_title: '独立专项' });
  expect(ownerLabel(flattenTodos(snapshot)[0])).toBe('独立专项');
  expect(milestoneOptions(snapshot)[0]).toMatchObject({ value: 'm1' });
});

it('creates a standalone milestone without any project and navigates its TODO list', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '里程碑' }));
  await screen.findByText('独立专项', { exact: true });
  fireEvent.click(button('新建里程碑'));
  fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '新独立专项' } });
  fireEvent.click(button('保存'));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/milestones', expect.objectContaining({ title: '新独立专项', goal_id: null })));
  // jsdom does not complete CSS transitions; the leave state proves the save closed it.
  await waitFor(() => expect(document.querySelector('.ant-modal')?.className).toContain('leave'));
  fireEvent.animationEnd(document.querySelector('.ant-modal'));
  fireEvent.click(button('1条TODO'));
  await screen.findByText('专项任务', { exact: true });
  expect(screen.queryByText('独立任务', { exact: true })).toBeNull();
});

it('searches associations by label or ID, sends a single ID and clears explicitly', async () => {
  const props = { path: '/api/project/todos/t', field: 'milestone_id', value: null,
    options: [{ value: 'm1', label: '同名专项 · 项目 A · m1' }, { value: 'm2', label: '同名专项 · 独立 · m2' }], refresh: vi.fn(), onNotice: vi.fn(), label: '选择里程碑' };
  const view = render(<RelationSelect {...props} />);
  const select = screen.getByRole('combobox', { name: '选择里程碑' });
  fireEvent.change(select, { target: { value: 'm2' } });
  fireEvent.mouseDown(select);
  fireEvent.click(await screen.findByText('同名专项 · 独立 · m2', { selector: '.ant-select-item-option-content' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith(props.path, { milestone_id: 'm2' }));
  view.rerender(<RelationSelect {...props} value="m2" />);
  await waitFor(() => expect(screen.getByRole('combobox', { name: '选择里程碑' }).disabled).toBe(false));
  fireEvent.click(document.querySelector('.ant-select-clear'));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith(props.path, { milestone_id: null }));
});
