// @vitest-environment jsdom
import { App, ConfigProvider } from 'antd';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
import { DraftGuardProvider } from '../../ontology/navigation/DraftGuard';
import { setIdentity } from '../../store.js';
import { ProjectPanel } from '../project.jsx';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
const todo = { id: 't1', title: '分享任务', draft: '# 任务内容', board_status: 'todo', tag_ids: [] };
const initiative = { id: 'i1', title: '分享子项', todos: [todo] };
const overview = { goals: [{ id: 'p1', title: '分享项目', initiatives: [initiative] }], standalone_initiatives: [], backlog: [], tags: [] };
const mount = () => render(<ConfigProvider theme={{ token: { motion: false } }}><App><DraftGuardProvider><ProjectPanel onNotice={vi.fn()} /></DraftGuardProvider></App></ConfigProvider>);
// Antd's test build reuses "test-id" across labels. Resolve the actual drawer
// heading here; real accessible names are checked by the browser acceptance.
const heading = { selector: '.ant-drawer-title, .ant-modal-confirm-title' };
const dialog = async (name) => (await screen.findByText(name, heading)).closest('[role="dialog"]');
const queryDialog = (name) => screen.queryByText(name, heading)?.closest('[role="dialog"]') || null;
beforeEach(() => {
  window.history.replaceState(null, '', '/');
  setIdentity({ name: 'reader', role: 'editor' });
  vi.resetAllMocks();
  api.apiGet.mockImplementation(async (path) => path === '/api/project/overview' ? overview : path.endsWith('/people') ? { people: [] }
    : path.includes('/comments') ? { comments: [], has_more: false } : { assignments: [], capabilities: [] });
});
afterEach(() => window.history.replaceState(null, '', '/'));

it.each(['/#todo=t1', '/?todo=t1', '/#initiative=i1'])('closes an ID-only %s link through its real ancestors', async (href) => {
  window.history.replaceState(null, '', href);
  mount();
  if (href.includes('todo=')) {
    const task = await dialog('TODO · 分享任务');
    expect(queryDialog('子项 · 分享子项')).toBeNull();
    expect(queryDialog('项目 · 分享项目')).toBeNull();
    fireEvent.click(task.querySelector('.ant-drawer-close'));
    await waitFor(() => expect(queryDialog('TODO · 分享任务')).toBeNull());
    expect(window.location.hash).toBe('#project=p1&initiative=i1');
  }
  fireEvent.click((await dialog('子项 · 分享子项')).querySelector('.ant-drawer-close'));
  await waitFor(() => expect(queryDialog('子项 · 分享子项')).toBeNull());
  expect(window.location.hash).toBe('#project=p1');
  fireEvent.click((await dialog('项目 · 分享项目')).querySelector('.ant-drawer-close'));
  await waitFor(() => expect(window.location.hash).toBe(''));
  expect(api.apiPost).not.toHaveBeenCalled();
  expect(api.apiPatch).not.toHaveBeenCalled();
});

it('restores ancestors after delayed loading without replacing the shared address', async () => {
  let resolve;
  api.apiGet.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
  window.history.replaceState(null, '', '/#todo=t1');
  mount(); await dialog('TODO · t1');
  expect(queryDialog('项目 · 分享项目')).toBeNull();
  await act(async () => resolve(overview));
  await dialog('TODO · 分享任务');
  expect(queryDialog('子项 · 分享子项')).toBeNull();
  expect(queryDialog('项目 · 分享项目')).toBeNull();
  expect(window.location.hash).toBe('#todo=t1');
});

// Escape ordering is exercised against the production build in navigation.js;
// Antd's test-only shared portal ID prevents a faithful keyboard stack here.
it.each([['project=p1', '项目 · 分享项目'], ['initiative=i1', '子项 · 分享子项'], ['todo=t1', 'TODO · 分享任务']])('opens and restores a shared %s link without earlier navigation', async (hash, name) => {
  window.history.replaceState(null, '', '/#' + hash);
  setIdentity({ name: 'reader', role: 'viewer' });
  const view = mount();
  await dialog(name);
  view.unmount();
  mount();
  await dialog(name);
  expect(window.location.hash).toBe('#' + hash);
  expect(api.apiPost).not.toHaveBeenCalled();
  expect(api.apiPatch).not.toHaveBeenCalled();
});

async function openNested() {
  fireEvent.click((await screen.findByText('分享项目', { exact: true })).closest('button'));
  expect(window.location.hash).toBe('#project=p1');
  const project = await dialog('项目 · 分享项目');
  fireEvent.click(within(project).getByRole('tab', { name: '子项', exact: true }));
  fireEvent.click(within(project).getByText('分享子项', { exact: true }).closest('button'));
  expect(window.location.hash).toBe('#project=p1&initiative=i1');
  const group = await dialog('子项 · 分享子项');
  fireEvent.click(within(group).getByText('分享任务', { exact: true }).closest('button'));
  await dialog('TODO · 分享任务');
  expect(window.location.hash).toBe('#project=p1&initiative=i1&todo=t1');
}

it('keeps nested drawer addresses in sync with Back, Forward and closing', async () => {
  mount(); await openNested();
  act(() => window.history.back());
  await waitFor(() => expect(queryDialog('TODO · 分享任务')).toBeNull());
  expect(window.location.hash).toBe('#project=p1&initiative=i1');
  act(() => window.history.forward());
  const task = await dialog('TODO · 分享任务');
  fireEvent.click(task.querySelector('.ant-drawer-close'));
  await waitFor(() => expect(window.location.hash).toBe('#project=p1&initiative=i1'));
  const group = await dialog('子项 · 分享子项');
  fireEvent.click(group.querySelector('.ant-drawer-close'));
  await waitFor(() => expect(window.location.hash).toBe('#project=p1'));
  const project = await dialog('项目 · 分享项目');
  fireEvent.click(project.querySelector('.ant-drawer-close'));
  await waitFor(() => expect(window.location.hash).toBe(''));
}, 20000);

it('keeps an unsaved TODO and its URL on cancelled Back, then supports confirmed Back and Forward', async () => {
  mount(); await openNested();
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '未保存标题' } });
  act(() => window.history.back());
  const confirm = await dialog('有内容尚未保存');
  expect(window.location.hash).toBe('#project=p1&initiative=i1&todo=t1');
  fireEvent.click(within(confirm).getByRole('button', { name: '继续编辑' }));
  expect(screen.getByLabelText('TODO 标题').value).toBe('未保存标题');
  await waitFor(() => expect(queryDialog('有内容尚未保存')).toBeNull());
  act(() => window.history.back());
  fireEvent.click(within(await dialog('有内容尚未保存')).getByRole('button', { name: '放弃并继续' }));
  await waitFor(() => expect(queryDialog('TODO · 分享任务')).toBeNull());
  expect(window.location.hash).toBe('#project=p1&initiative=i1');
  act(() => window.history.forward());
  await dialog('TODO · 分享任务');
  expect(screen.getByLabelText('TODO 标题').value).toBe('分享任务');
  expect(api.apiPatch).not.toHaveBeenCalled();
}, 20000);

it('retains the destination through load failures and retry, and reports missing records separately', async () => {
  window.history.replaceState(null, '', '/?keep=1#todo=t1');
  api.apiGet.mockRejectedValueOnce(new Error('暂时无法连接'));
  mount();
  const task = await dialog('TODO · t1');
  await within(task).findByText('无法读取TODO');
  expect(within(task).queryByText('TODO不存在或已被删除')).toBeNull();
  fireEvent.click(within(task).getByRole('button', { name: /重\s*试/ }));
  await dialog('TODO · 分享任务');
  act(() => { window.location.hash = 'todo=missing'; });
  const missing = await dialog('TODO · missing');
  expect(within(missing).getByText('TODO不存在或已被删除')).toBeTruthy();
  fireEvent.click(missing.querySelector('.ant-drawer-close'));
  await waitFor(() => expect(window.location.hash).toBe(''));
  expect(window.location.search).toBe('?keep=1');
});

it('preserves a comment draft when a different shared link is cancelled', async () => {
  window.history.replaceState(null, '', '/#todo=t1');
  mount(); await dialog('TODO · 分享任务');
  fireEvent.change(screen.getByLabelText('评论正文'), { target: { value: '未发送的评论' } });
  act(() => { window.location.hash = 'initiative=i1'; });
  fireEvent.click(within(await dialog('有内容尚未保存')).getByRole('button', { name: '继续编辑' }));
  expect(window.location.hash).toBe('#todo=t1');
  expect(screen.getByLabelText('评论正文').value).toBe('未发送的评论');
  expect(api.apiPost).not.toHaveBeenCalled();
});
