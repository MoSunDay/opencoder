// @vitest-environment jsdom
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const dnd = vi.hoisted(() => ({ end: null }));
vi.mock('@dnd-kit/core', async (original) => ({ ...(await original()), DndContext: ({ children, onDragEnd }) => { dnd.end = onDragEnd; return children; } }));
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPut: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
import { apiGet, apiPut } from '../../api.js';
import { setIdentity } from '../../store.js';
import { ProjectInitiatives } from './initiatives.jsx';
import { ProjectsTab } from '../projectsTab.jsx';
import { InitiativesTab } from '../initiativesTab.jsx';

const rows = ['a', 'hidden', 'b'].map((id) => ({ id, title: id === 'hidden' ? '隐藏子项' : `可见子项 ${id}`,
  todos: [], progress: { total: 0, done: 0, percent: 0 }, goal_title: '项目', summary_updated_at: 1000 }));
const props = () => ({ projectId: 'p', rows, refresh: vi.fn(), onNotice: vi.fn(), openInitiative: vi.fn() });
const drag = (active, over) => dnd.end({ active: { id: active }, over: over ? { id: over } : null });
const order = () => [...screen.getByLabelText('项目内子项表格').querySelectorAll('tr[data-row-key]')].map((row) => row.dataset.rowKey);
beforeEach(() => {
  vi.resetAllMocks(); setIdentity({ name: 'editor', role: 'editor' });
  apiPut.mockResolvedValue({ ok: true });
  apiGet.mockResolvedValue({ versions: [{ version: 1, current_summary: '历史正文', created_at: 1000 }] });
});

it('saves a filtered drag with the full order and prevents repeated saves', async () => {
  let finish;
  apiPut.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const p = props(); const view = render(<ProjectInitiatives {...p} />);
  fireEvent.click(screen.getAllByLabelText('筛选子项')[0]);
  fireEvent.change(screen.getByRole('textbox', { name: '搜索子项' }), { target: { value: '可见' } });
  fireEvent.click(screen.getByRole('button', { name: /搜\s*索/ }));
  expect(order()).toEqual(['a', 'b']);
  await act(async () => { drag('a', 'b'); });
  expect(order()).toEqual(['b', 'a']);
  expect(apiPut).toHaveBeenCalledWith('/api/project/goals/p/initiatives/order', { expected_ids: ['a', 'hidden', 'b'], ids: ['hidden', 'b', 'a'] });
  await act(async () => { drag('b', 'a'); });
  expect(apiPut).toHaveBeenCalledOnce();
  view.rerender(<ProjectInitiatives {...p} rows={[rows[1], rows[2], rows[0]]} />);
  await act(async () => finish({ ok: true }));
  expect(p.refresh).toHaveBeenCalledOnce();
  expect(order()).toEqual(['b', 'a']);
  expect(p.openInitiative).not.toHaveBeenCalled();
});

it('restores the original order after failure and ignores cancelled or read-only drags', async () => {
  apiPut.mockRejectedValueOnce(new Error('write rejected'));
  const p = props(); render(<ProjectInitiatives {...p} />);
  await act(async () => drag('a', null));
  expect(apiPut).not.toHaveBeenCalled();
  await act(async () => drag('a', 'b'));
  expect(order()).toEqual(['a', 'hidden', 'b']);
  expect(p.onNotice).toHaveBeenCalledWith(expect.objectContaining({ text: expect.stringContaining('write rejected') }));
  act(() => setIdentity({ name: 'viewer', role: 'viewer' }));
  for (const handle of screen.getAllByRole('button', { name: /^拖动子项/ })) expect(handle.disabled).toBe(true);
  await act(async () => drag('b', 'a'));
  expect(apiPut).toHaveBeenCalledOnce();
});

it.each(['projects', 'initiatives', 'project-initiatives'])('opens history from %s without opening the row detail, including for viewers', async (kind) => {
  setIdentity({ name: 'viewer', role: 'viewer' });
  const p = props(); const open = vi.fn();
  const overview = { goals: [{ id: 'p', title: '项目', status: 'active', initiatives: [rows[0]] }], standalone_initiatives: [] };
  render(kind === 'projects' ? <ProjectsTab overview={overview} refresh={p.refresh} onNotice={p.onNotice} openProject={open} />
    : kind === 'initiatives' ? <InitiativesTab overview={overview} refresh={p.refresh} onNotice={p.onNotice} openInitiative={open} />
      : <ProjectInitiatives {...p} rows={[rows[0]]} openInitiative={open} />);
  fireEvent.click(screen.getByRole('button', { name: '现状历史' }));
  const history = await screen.findByRole('dialog', { name: '现状修改记录' });
  expect(await within(history).findByText('历史正文')).toBeTruthy();
  fireEvent.click(within(history).getByText('历史正文'));
  expect(open).not.toHaveBeenCalled();
  expect(apiGet.mock.calls[0][0]).toBe(`/api/project/${kind === 'projects' ? 'goals/p' : 'initiatives/a'}/summary-versions`);
});
