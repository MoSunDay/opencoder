// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { ProjectTagsPanel } from './panel.jsx';
import { apiGet, apiPost } from '../../api.js';
import { setState } from '../../store.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
afterEach(() => { cleanup(); setState({ identity: null }); vi.resetAllMocks(); });
const overview = { goals: [{ id: 'p', title: '项目一', initiatives: [] }], standalone_initiatives: [], backlog: [],
  tags: [{ id: 'tag', name: '前端', scope_type: 'project', scope_id: 'p' }] };

it('opens the standalone Tag table and reloads it after saving', async () => {
  apiGet.mockResolvedValue(overview); apiPost.mockResolvedValue({ id: 'created' });
  render(<ProjectTagsPanel onNotice={vi.fn()} />);
  expect(await screen.findByText('前端')).toBeTruthy();
  expect(screen.queryByRole('tab')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '新建 Tag' }));
  fireEvent.change(screen.getByLabelText('Tag 名称'), { target: { value: '后端' } });
  fireEvent.mouseDown(screen.getByLabelText('Tag 归属'));
  fireEvent.click((await screen.findAllByText('项目一')).find((el) => el.classList.contains('ant-select-item-option-content')));
  fireEvent.click(screen.getByRole('button', { name: '保存 Tag' }));
  await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/project/tags', { name: '后端', scope_type: 'project', scope_id: 'p' }));
  expect(apiGet.mock.calls.filter(([path]) => path === '/api/project/overview').length).toBeGreaterThan(1);
});

it('failed initial loading shows the error and can retry instead of showing an empty table', async () => {
  apiGet.mockRejectedValueOnce(new Error('连接中断')).mockResolvedValue(overview);
  render(<ProjectTagsPanel onNotice={vi.fn()} />);
  expect(await screen.findByText('获取项目总览失败: 连接中断')).toBeTruthy();
  expect(screen.queryByLabelText('Tag 表格')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
  expect(await screen.findByText('前端')).toBeTruthy();
});

it('viewer can open Tag but cannot create, edit or delete definitions', async () => {
  setState({ identity: { name: 'reader', role: 'viewer' } }); apiGet.mockResolvedValue(overview);
  render(<ProjectTagsPanel onNotice={vi.fn()} />);
  await screen.findByText('前端');
  for (const name of ['新建 Tag', '编辑 Tag 前端', '删除 Tag 前端']) expect(screen.getByRole('button', { name }).disabled).toBe(true);
});
