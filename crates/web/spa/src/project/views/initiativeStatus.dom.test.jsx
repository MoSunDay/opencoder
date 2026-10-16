// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
import { InitiativesTab } from '../initiativesTab.jsx';
import { InitiativeDrawer } from './initiativeDrawer.jsx';
import { ProjectDrawer } from './projectDrawer.jsx';

const initiative = { id: 'i', goal_id: 'p', title: '子项甲', current_summary: '现状', detail_md: '原子项目标', todos: [
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
  const table = within(screen.getByLabelText('子项表格'));
  expect(table.queryByText('状态')).toBeNull();
  expect(table.getByText('子项甲')).toBeTruthy();
  expect(table.getByText('100%')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '新建子项' }));
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
  expect(screen.queryByRole('tab')).toBeNull();
  expect(screen.getByLabelText('现状')).toBeTruthy();
  const context = screen.getByLabelText('现状').closest('.project-initiative-context');
  const meta = document.querySelector('.project-initiative-drawer .project-detail-meta');
  expect(context.compareDocumentPosition(meta) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(within(meta).getByText('所属项目：项目甲')).toBeTruthy();
  expect(screen.getByLabelText('TODO 看板')).toBeTruthy();
  expect(screen.getByText('保留 TODO 状态')).toBeTruthy();
  expect(screen.getAllByText(/已完成/).length).toBeGreaterThan(0);
  fireEvent.click(screen.getByRole('button', { name: '编辑子项' }));
  expect(screen.queryByLabelText('状态')).toBeNull();
  const description = screen.getByRole('textbox', { name: '子项目标', exact: true });
  expect(description.value).toBe('原子项目标');
  fireEvent.change(description, { target: { value: '## 交付范围\n\n**子项目标已补齐**' } });
  fireEvent.click(screen.getByRole('button', { name: /^保\s*存$/ }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledTimes(1));
  expect(api.apiPatch.mock.calls[0][1]).not.toHaveProperty('status');
  expect(api.apiPatch.mock.calls[0][1].detail_md).toContain('子项目标已补齐');
  expect(api.apiPatch.mock.calls[0][1]).not.toHaveProperty('current_summary');
});

it('project detail creates initiatives without state', async () => {
  render(<ProjectDrawer {...props} projectId="p" />);
  fireEvent.click(screen.getByRole('tab', { name: '子项', exact: true }));
  expect(within(screen.getByLabelText('项目内子项表格')).queryByText('状态')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '新建子项' }));
  fireEvent.change(screen.getByLabelText('标题'), { target: { value: '项目内子项' } });
  fireEvent.click(screen.getByRole('button', { name: /^保\s*存$/ }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledTimes(1));
  expect(api.apiPost.mock.calls[0][1]).not.toHaveProperty('status');
  expect(api.apiPost.mock.calls[0][1].goal_id).toBe('p');
});

it.each([
  ['项目目标', ProjectDrawer, { projectId: 'p' }],
  ['子项目标', InitiativeDrawer, { initiativeId: 'i' }],
])('%s and summary collapse independently and retain expansion after refresh', (description, DrawerView, selection) => {
  const data = { ...overview, goals: [{ ...overview.goals[0], current_summary: '## 项目进展', detail_md: '## 项目范围' }] };
  const view = render(<DrawerView {...props} {...selection} overview={data} />);
  const body = (name) => document.getElementById(screen.getByRole('button', { name }).getAttribute('aria-controls'));
  expect(body('展开现状').hidden).toBe(true);
  expect(body(`展开${description}`).hidden).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '展开现状' }));
  expect(body('收起现状').hidden).toBe(false);
  expect(body(`展开${description}`).hidden).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: `展开${description}` }));
  expect(body(`收起${description}`).hidden).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: '收起现状' }));
  expect(body('展开现状').hidden).toBe(true);
  view.rerender(<DrawerView {...props} {...selection} overview={{ ...data }} />);
  expect(body(`收起${description}`).hidden).toBe(false);
  expect(api.apiPatch).not.toHaveBeenCalled();
});

it('resets summary and description folds when switching initiatives', () => {
  const data = { ...overview, standalone_initiatives: [{ ...initiative, id: 'other', goal_id: null }] };
  const view = render(<InitiativeDrawer {...props} overview={data} initiativeId="i" />);
  fireEvent.click(screen.getByRole('button', { name: '展开现状' }));
  fireEvent.click(screen.getByRole('button', { name: '展开子项目标' }));
  view.rerender(<InitiativeDrawer {...props} overview={data} initiativeId="other" />);
  expect(screen.getByRole('button', { name: '展开现状' }).getAttribute('aria-expanded')).toBe('false');
  expect(screen.getByRole('button', { name: '展开子项目标' }).getAttribute('aria-expanded')).toBe('false');
});

it('ignores Escape already consumed by a child interaction and still closes on ordinary Escape', () => {
  const onClose = vi.fn();
  render(<InitiativeDrawer {...props} initiativeId="i" onClose={onClose} />);
  const consumed = new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', bubbles: true, cancelable: true });
  consumed.preventDefault();
  fireEvent(window, consumed);
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.keyDown(window, { key: 'Escape', code: 'Escape' });
  expect(onClose).toHaveBeenCalledOnce();
});
