// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { ProjectPanel } from '../project.jsx';
import { flattenTodos, flattenInitiatives, groupOptions, initiativeOptions } from '../model/relations.js';

const snapshot = { goals: [], standalone_initiatives: [{ id: 'm1', goal_id: null, title: '独立子项', todos: [
  { id: 't1', title: '子项任务', draft: 'd', status: 'draft', initiative_id: 'm1' },
] }], backlog: [{ id: 't2', title: '独立任务', draft: 'd', status: 'draft' }] };
const button = (label) => [...document.querySelectorAll('button')].find((b) => b.textContent.replace(/\s/g, '') === label);
beforeEach(() => { Object.values(api).forEach((fn) => fn.mockReset()); api.apiGet.mockResolvedValue(snapshot); api.apiPost.mockResolvedValue({ id: 'm2' }); api.apiPatch.mockResolvedValue({ ok: true }); });

it('projects, selectors and rollups include independent work exactly once', () => {
  expect(flattenTodos(snapshot).map((t) => t.id)).toEqual(['t1', 't2']);
  expect(flattenInitiatives(snapshot)).toHaveLength(1);
  expect(flattenInitiatives(snapshot)[0].todos).toHaveLength(1);
  expect(initiativeOptions(snapshot)[0]).toMatchObject({ value: 'm1' });
});

it('offers only initiatives as TODO parents', () => {
  const groups = { goals: [{ id: 'p1', title: '项目', initiatives: [{ id: 'i1', title: '子项' }] }], standalone_initiatives: [], backlog: [] };
  expect(groupOptions(groups).map((item) => item.value)).toEqual(['i1']);
});

it('creates a standalone initiative without any project and navigates its TODO list', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '子项' }));
  await screen.findByText('独立子项', { exact: true });
  fireEvent.click(button('新建子项'));
  fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '新独立子项' } });
  fireEvent.click(button('保存'));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/initiatives', expect.objectContaining({ title: '新独立子项', goal_id: null })));
  fireEvent.click(await screen.findByRole('button', { name: '独立子项' }));
  await screen.findByText('子项任务', { exact: true });
  expect(screen.queryByText('独立任务', { exact: true })).toBeNull();
}, 60000);
