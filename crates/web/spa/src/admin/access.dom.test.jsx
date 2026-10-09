// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import '../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
import { UsersPanel } from './users.jsx';
beforeEach(() => {
  Object.values(api).forEach((fn) => fn.mockReset());
  api.apiGet.mockResolvedValue({ users: [{ name: 'admin', role: 'admin' }, { name: 'alice', role: 'viewer' }] });
});
afterEach(cleanup);
const create = () => {
  fireEvent.click(screen.getByRole('button', { name: '创建用户' }));
  return within(screen.getByRole('dialog'));
};
it('keeps forms behind actions and protects the startup administrator', async () => {
  api.apiPost.mockResolvedValue({ user: { name: 'bob', role: 'viewer' } });
  render(<UsersPanel />);
  expect(await screen.findByText('admin · 启动管理员')).toBeTruthy();
  expect(screen.queryByLabelText('用户名')).toBeNull();
  expect(screen.queryByRole('combobox', { name: '用户角色' })).toBeNull();
  expect(screen.getAllByRole('button', { name: '编辑权限' })).toHaveLength(1);
  expect(screen.getAllByRole('button', { name: '删除用户' })).toHaveLength(1);
  const dialog = create();
  fireEvent.change(dialog.getByLabelText('用户名'), { target: { value: 'bob' } });
  fireEvent.click(dialog.getByRole('button', { name: '创建用户' }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/users', { name: 'bob', role: 'viewer' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
});
it('validates required fields and clears a cancelled creation form', async () => {
  render(<UsersPanel />);
  await screen.findByText('alice');
  const dialog = create();
  fireEvent.click(dialog.getByRole('button', { name: '创建用户' }));
  expect(await dialog.findByText('请输入用户名')).toBeTruthy();
  expect(api.apiPost).not.toHaveBeenCalled();
  fireEvent.change(dialog.getByLabelText('用户名'), { target: { value: 'draft' } });
  fireEvent.click(dialog.getByRole('button', { name: /取\s*消/ }));
  expect(create().getByLabelText('用户名').value).toBe('');
});
it('keeps failed creation inside the dialog with the draft available for retry', async () => {
  api.apiPost.mockRejectedValueOnce(new Error('用户名已存在')).mockResolvedValueOnce({});
  render(<UsersPanel />);
  await screen.findByText('alice');
  const dialog = create();
  fireEvent.change(dialog.getByLabelText('用户名'), { target: { value: 'bob' } });
  fireEvent.click(dialog.getByRole('button', { name: '创建用户' }));
  expect(await dialog.findByText('用户名已存在')).toBeTruthy();
  expect(dialog.getByLabelText('用户名').value).toBe('bob');
  fireEvent.click(dialog.getByRole('button', { name: '创建用户' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(api.apiPost).toHaveBeenCalledTimes(2);
});
it('saves a role only after confirmation and prevents duplicate submission', async () => {
  let resolve;
  api.apiPatch.mockImplementation(() => new Promise((done) => { resolve = done; }));
  render(<UsersPanel />);
  await screen.findByText('alice');
  fireEvent.click(screen.getByRole('button', { name: '编辑权限' }));
  const dialog = within(screen.getByRole('dialog'));
  expect(dialog.getByText('编辑权限 · alice')).toBeTruthy();
  expect(dialog.getByLabelText('用户名').disabled).toBe(true);
  fireEvent.mouseDown(dialog.getByRole('combobox', { name: '用户角色' }));
  fireEvent.click(await screen.findByText('editor · 可编辑', { selector: '.ant-select-item-option-content' }));
  expect(api.apiPatch).not.toHaveBeenCalled();
  fireEvent.click(dialog.getByRole('button', { name: '保存权限' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/users/alice', { role: 'editor' }));
  expect(dialog.getByRole('button', { name: /取\s*消/ }).disabled).toBe(true);
  fireEvent.submit(dialog.getByLabelText('用户名').closest('form'));
  expect(api.apiPatch).toHaveBeenCalledTimes(1);
  resolve({});
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
});
it('combines user search and role filtering and exposes no-match feedback', async () => {
  render(<UsersPanel />);
  await screen.findByText('alice');
  fireEvent.change(screen.getByLabelText('搜索用户名'), { target: { value: ' ALI ' } });
  expect(screen.getByText('alice')).toBeTruthy();
  expect(screen.queryByText('admin · 启动管理员')).toBeNull();
  fireEvent.mouseDown(screen.getByRole('combobox', { name: '筛选用户角色' }));
  fireEvent.click(await screen.findByText('editor · 可编辑', { selector: '.ant-select-item-option-content' }));
  expect(screen.queryByText('alice')).toBeNull();
  expect(screen.getByText('没有匹配的用户')).toBeTruthy();
});
it('waits for delete confirmation and reports a failed delete without removing the user', async () => {
  api.apiDel.mockRejectedValue(new Error('删除失败'));
  render(<UsersPanel />);
  await screen.findByText('alice');
  fireEvent.click(screen.getByRole('button', { name: '删除用户' }));
  expect(api.apiDel).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByRole('button', { name: '确认删除' }));
  await screen.findByText('删除失败');
  expect(api.apiDel).toHaveBeenCalledWith('/api/users/alice');
  expect(screen.getByText('alice')).toBeTruthy();
});
it('keeps list errors visible with a working retry', async () => {
  api.apiGet.mockRejectedValueOnce(new Error('读取失败'));
  render(<UsersPanel />);
  await screen.findByText('读取失败');
  fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
  expect(await screen.findByText('alice')).toBeTruthy();
  expect(screen.queryByText('读取失败')).toBeNull();
});
