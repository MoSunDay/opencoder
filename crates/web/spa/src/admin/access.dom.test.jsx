// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import '../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
import { UsersPanel } from './users.jsx';
import { TokensPanel } from './tokens.jsx';
beforeEach(() => {
  Object.values(api).forEach((fn) => fn.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/users'
    ? { users: [{ name: 'admin', role: 'admin' }, { name: 'alice', role: 'viewer' }] }
    : { tokens: [{ id: 'one', user_name: 'alice', name: 'CLI', created_at: 1, expires_at: null }] }));
});
afterEach(cleanup);
it('shows a protected startup administrator and creates a viewer without exposing credentials', async () => {
  api.apiPost.mockResolvedValue({ user: { name: 'bob', role: 'viewer' } });
  render(<UsersPanel />);
  expect(await screen.findByText('admin · 启动管理员')).toBeTruthy();
  expect(screen.getAllByRole('button', { name: '删除用户' })).toHaveLength(1);
  fireEvent.change(screen.getByLabelText('用户名'), { target: { value: 'bob' } });
  fireEvent.click(screen.getByRole('button', { name: '创建用户' }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/users', { name: 'bob', role: 'viewer' }));
});
it('issues an optional-expiry token and removes plaintext after acknowledgment', async () => {
  api.apiPost.mockResolvedValue({ token: 'oc_once_only', metadata: { id: 'new' } });
  render(<TokensPanel />);
  await screen.findByText('不过期');
  fireEvent.mouseDown(screen.getByRole('combobox', { name: 'Token 所属用户' }));
  fireEvent.click(await screen.findByText('alice · viewer'));
  fireEvent.change(screen.getByLabelText('Token 名称'), { target: { value: 'automation' } });
  fireEvent.click(screen.getByRole('button', { name: '创建 Token' }));
  await screen.findByText('oc_once_only');
  expect(api.apiPost).toHaveBeenCalledWith('/api/tokens', { user_name: 'alice', name: 'automation', expires_at: null });
  fireEvent.click(screen.getByRole('button', { name: '我已保存' }));
  await waitFor(() => expect(screen.queryByText('oc_once_only')).toBeNull());
});
it('keeps a failed list visible as an error with retry', async () => {
  api.apiGet.mockRejectedValue(new Error('读取失败'));
  render(<TokensPanel />);
  await screen.findByText('读取失败');
  expect(screen.getByRole('button', { name: /重\s*试/ })).toBeTruthy();
});
