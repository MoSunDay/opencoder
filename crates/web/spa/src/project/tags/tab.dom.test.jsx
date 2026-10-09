// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
import { TagsTab } from './tab.jsx';
import { ProjectViewState } from '../views/viewState.jsx';

const overview = {
  goals: [{ id: 'p', title: '项目甲', initiatives: [{ id: 'i', title: '专项甲' }] }, { id: 'q', title: '项目乙', initiatives: [] }],
  standalone_initiatives: [{ id: 's', title: '独立专项甲' }],
  tags: [{ id: 'parent', name: '前端', scope_type: 'project', scope_id: 'p' }, { id: 'local', name: '前端', scope_type: 'initiative', scope_id: 'i' }],
};
const props = () => ({ overview, refresh: vi.fn(), onNotice: vi.fn() });
async function select(label, option) {
  fireEvent.mouseDown(screen.getByRole('combobox', { name: label }));
  fireEvent.click(await screen.findByText(option, { selector: '.ant-select-item-option-content', exact: true }));
}
const save = () => fireEvent.click(screen.getByRole('button', { name: /保存 Tag/ }));
beforeEach(() => { Object.values(api).forEach((method) => method.mockReset()); api.apiPost.mockResolvedValue({ id: 'new' }); api.apiPatch.mockResolvedValue({ id: 'parent' }); api.apiDel.mockResolvedValue({ deleted: true }); });

it('lists both same-name definitions and creates a tag on a standalone initiative', async () => {
  const p = props(); render(<TagsTab {...p} />);
  const table = screen.getByLabelText('Tag 表格');
  expect(within(table).getAllByText('前端')).toHaveLength(2);
  fireEvent.click(screen.getByRole('button', { name: '新建 Tag' }));
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: ' 独立标签 ' } });
  await select('Tag 归属类型', '专项'); await select('Tag 归属', '独立专项甲 · 独立专项'); save();
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/tags', { name: '独立标签', scope_type: 'initiative', scope_id: 's' }));
  expect(p.refresh).toHaveBeenCalledOnce();
});

it('clears the old owner on type change and saves name and scope using the original ID', async () => {
  render(<TagsTab {...props()} />);
  fireEvent.click(screen.getAllByRole('button', { name: '编辑 Tag 前端' })[1]);
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: '后端' } });
  await select('Tag 归属类型', '专项'); save();
  await screen.findByText('请选择具体项目或专项'); expect(api.apiPatch).not.toHaveBeenCalled();
  await select('Tag 归属', '专项甲 · 项目甲'); save();
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/tags/parent', { name: '后端', scope_type: 'initiative', scope_id: 'i' }));
});

it('preserves the draft after failure and submits only once while a write is pending', async () => {
  let reject; api.apiPatch.mockImplementationOnce(() => new Promise((_, failure) => { reject = failure; }));
  const p = props(); const view = render(<TagsTab {...p} />);
  fireEvent.click(screen.getAllByRole('button', { name: '编辑 Tag 前端' })[1]);
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: '未保存名称' } });
  view.rerender(<TagsTab {...p} overview={{ ...overview, tags: overview.tags.map((tag) => ({ ...tag })) }} />);
  expect(screen.getByLabelText('Tag 名称').value).toBe('未保存名称');
  save(); save(); await waitFor(() => expect(api.apiPatch).toHaveBeenCalledOnce());
  expect(screen.getByLabelText('Tag 名称').disabled).toBe(true);
  await act(async () => reject(new Error('名称重复')));
  await waitFor(() => expect(screen.getByLabelText('Tag 名称').disabled).toBe(false));
  expect(p.onNotice).toHaveBeenCalledWith({ type: 'error', text: '名称重复' });
  expect(screen.getByLabelText('Tag 名称').value).toBe('未保存名称');
  expect(p.refresh).not.toHaveBeenCalled();
  save(); await waitFor(() => expect(api.apiPatch).toHaveBeenCalledTimes(2));
});

it('rejects blank and overlong names and missing owners', async () => {
  render(<TagsTab {...props()} />); fireEvent.click(screen.getByRole('button', { name: '新建 Tag' }));
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: '   ' } }); save();
  await screen.findByText('请输入 1–128 字符的 Tag 名称');
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: '长'.repeat(129) } }); save();
  await screen.findByText('请选择具体项目或专项'); expect(api.apiPost).not.toHaveBeenCalled();
});

it('keeps filters after saving and requires confirmation before deleting', async () => {
  const p = props(); const view = render(<ProjectViewState><TagsTab {...p} /></ProjectViewState>);
  const table = screen.getByLabelText('Tag 表格');
  fireEvent.click(within(table).getByLabelText('筛选所属专项'));
  const filter = await screen.findByLabelText('搜索所属专项');
  fireEvent.change(filter, { target: { value: '专项甲' } }); fireEvent.keyDown(filter, { key: 'Enter', code: 'Enter' });
  await waitFor(() => expect(within(table).getAllByText('前端')).toHaveLength(1));
  fireEvent.click(within(table).getByRole('button', { name: '编辑 Tag 前端' }));
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: '新前端' } }); save();
  await waitFor(() => expect(p.refresh).toHaveBeenCalledOnce());
  view.rerender(<ProjectViewState><TagsTab {...p} overview={{ ...overview, tags: overview.tags.map((tag) => tag.id === 'local' ? { ...tag, name: '新前端' } : tag) }} /></ProjectViewState>);
  expect(within(table).queryByText('前端')).toBeNull();
  fireEvent.click(within(table).getByRole('button', { name: '删除 Tag 新前端' }));
  expect(api.apiDel).not.toHaveBeenCalled();
  fireEvent.click(within(await screen.findByRole('tooltip')).getByRole('button', { name: /删.*除/ }));
  await waitFor(() => expect(api.apiDel).toHaveBeenCalledWith('/api/project/tags/local'));
});
