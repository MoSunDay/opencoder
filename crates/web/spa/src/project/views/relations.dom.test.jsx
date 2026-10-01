// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => null }));
import { ProjectPanel } from '../project.jsx';
import { RelationSelect } from './relationSelect.jsx';
import { flattenTodos, flattenInitiatives, groupOptions, initiativeOptions } from '../model/relations.js';

const snapshot = { goals: [], standalone_initiatives: [{ id: 'm1', goal_id: null, title: '独立专项', status: 'planned', todos: [
  { id: 't1', title: '专项任务', draft: 'd', status: 'draft', initiative_id: 'm1' },
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
  const groups = { goals: [{ id: 'p1', title: '项目', initiatives: [{ id: 'i1', title: '专项' }] }], standalone_initiatives: [], backlog: [] };
  expect(groupOptions(groups).map((item) => item.value)).toEqual(['i1']);
});

it('creates a standalone initiative without any project and navigates its TODO list', async () => {
  render(<ProjectPanel onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '专项' }));
  await screen.findByText('独立专项', { exact: true });
  fireEvent.click(button('新建专项'));
  fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '新独立专项' } });
  fireEvent.click(button('保存'));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/initiatives', expect.objectContaining({ title: '新独立专项', goal_id: null })));
  fireEvent.click(await screen.findByRole('button', { name: '独立专项' }));
  await screen.findByText('专项任务', { exact: true });
  expect(screen.queryByText('独立任务', { exact: true })).toBeNull();
}, 60000);

it('searches associations by label or ID, sends a single ID and clears explicitly', async () => {
  const props = { path: '/api/project/todos/t', field: 'initiative_id', value: null,
    options: [{ value: 'm1', label: '同名专项 · 项目 A · m1' }, { value: 'm2', label: '同名专项 · 独立 · m2' }], refresh: vi.fn(), onNotice: vi.fn(), label: '选择专项' };
  const view = render(<RelationSelect {...props} />);
  const select = screen.getByRole('combobox', { name: '选择专项' });
  fireEvent.change(select, { target: { value: 'm2' } });
  fireEvent.mouseDown(select);
  fireEvent.click(await screen.findByText('同名专项 · 独立 · m2', { selector: '.ant-select-item-option-content' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith(props.path, { initiative_id: 'm2' }));
  view.rerender(<RelationSelect {...props} value="m2" />);
  await waitFor(() => expect(screen.getByRole('combobox', { name: '选择专项' }).disabled).toBe(false));
  fireEvent.click(document.querySelector('.ant-select-clear'));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith(props.path, { initiative_id: null }));
});

it('isolates pending association and late save failure when the record changes', async () => {
  let rejectOld;
  api.apiPatch.mockImplementationOnce(() => new Promise((_, reject) => { rejectOld = reject; }));
  const props = { path: '/api/project/todos/old', field: 'initiative_id', value: 'm1',
    options: [{ value: 'm1', label: 'First' }, { value: 'm2', label: 'Second' }],
    refresh: vi.fn(), onNotice: vi.fn(), label: '选择专项' };
  const view = render(<RelationSelect {...props} />);
  fireEvent.mouseDown(screen.getByRole('combobox'));
  fireEvent.click(await screen.findByText('Second', { selector: '.ant-select-item-option-content' }));
  await waitFor(() => expect(screen.getByRole('combobox').disabled).toBe(true));
  view.rerender(<RelationSelect {...props} path="/api/project/todos/new" />);
  expect(screen.getByRole('combobox').disabled).toBe(false);
  expect(screen.getByText('First')).toBeTruthy();
  await act(async () => rejectOld(new Error('old save failed')));
  expect(screen.getByRole('combobox').disabled).toBe(false);
  expect(screen.getByText('First')).toBeTruthy();
  fireEvent.click(document.querySelector('.ant-select-clear'));
  await waitFor(() => expect(api.apiPatch).toHaveBeenLastCalledWith('/api/project/todos/new', { initiative_id: null }));
});
