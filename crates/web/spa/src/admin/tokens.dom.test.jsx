// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import '../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
import { TokensPanel } from './tokens.jsx';
const users = [{ name: 'admin', role: 'admin' }, { name: 'alice', role: 'viewer' }];
const tokens = [{ id: 'one', user_name: 'alice', name: 'CLI', created_at: 1, expires_at: null }];
beforeEach(() => {
  Object.values(api).forEach((fn) => fn.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/users' ? { users } : { tokens }));
});
afterEach(cleanup);
async function create() {
  await screen.findByText('不过期');
  fireEvent.click(screen.getByRole('button', { name: '创建 Token' }));
  const dialog = within(screen.getByRole('dialog'));
  fireEvent.mouseDown(dialog.getByRole('combobox', { name: 'Token 所属用户' }));
  fireEvent.click(await screen.findByText('alice · viewer', { selector: '.ant-select-item-option-content' }));
  fireEvent.change(dialog.getByLabelText('Token 名称'), { target: { value: 'automation' } });
  return dialog;
}
it('issues a token from a dialog and removes plaintext after acknowledgment', async () => {
  api.apiPost.mockResolvedValue({ token: 'oc_once_only', metadata: { id: 'new' } });
  render(<TokensPanel />);
  expect(screen.queryByLabelText('Token 名称')).toBeNull();
  const dialog = await create();
  fireEvent.click(dialog.getByRole('button', { name: '创建 Token' }));
  await screen.findByText('oc_once_only');
  expect(api.apiPost).toHaveBeenCalledWith('/api/tokens', { user_name: 'alice', name: 'automation', expires_at: null });
  expect(screen.queryByLabelText('Token 名称')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '我已保存' }));
  await waitFor(() => expect(screen.queryByText('oc_once_only')).toBeNull());
  expect(screen.queryByLabelText('Token 名称')).toBeNull();
});
it('preserves the token form after a failed issuance and can cancel without issuing again', async () => {
  api.apiPost.mockRejectedValue(new Error('签发失败'));
  render(<TokensPanel />);
  const dialog = await create();
  fireEvent.click(dialog.getByRole('button', { name: '创建 Token' }));
  expect(await dialog.findByText('签发失败')).toBeTruthy();
  expect(dialog.getByLabelText('Token 名称').value).toBe('automation');
  fireEvent.click(dialog.getByRole('button', { name: /取\s*消/ }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(api.apiPost).toHaveBeenCalledTimes(1);
});
it('searches token identifiers and filters lifecycle status', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/users' ? { users } : { tokens: [
    ...tokens, { id: 'expired-id', user_name: 'alice', name: 'old', created_at: 1, expires_at: 2 },
    { id: 'revoked-id', user_name: 'alice', name: 'revoked', created_at: 1, revoked_at: 3 },
  ] }));
  render(<TokensPanel />);
  await screen.findByText('old');
  fireEvent.change(screen.getByLabelText('搜索名称、用户或 Token ID'), { target: { value: 'expired-id' } });
  expect(screen.getByText('old')).toBeTruthy();
  expect(screen.queryByText('CLI')).toBeNull();
  fireEvent.mouseDown(screen.getByRole('combobox', { name: '筛选 Token 状态' }));
  fireEvent.click(await screen.findByText('已撤销', { selector: '.ant-select-item-option-content' }));
  expect(screen.getByText('没有匹配的 Token')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('搜索名称、用户或 Token ID'), { target: { value: '' } });
  expect(screen.getByText('revoked')).toBeTruthy();
  expect(screen.getByRole('button', { name: /撤\s*销/ }).disabled).toBe(true);
});
it('requires confirmation before revocation and retains the row on error', async () => {
  api.apiDel.mockRejectedValue(new Error('撤销失败'));
  render(<TokensPanel />);
  await screen.findByText('CLI');
  fireEvent.click(screen.getByRole('button', { name: /撤\s*销/ }));
  expect(api.apiDel).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByRole('button', { name: '确认撤销' }));
  expect(await screen.findByText('撤销失败')).toBeTruthy();
  expect(api.apiDel).toHaveBeenCalledWith('/api/tokens/one');
  expect(screen.getByText('CLI')).toBeTruthy();
});
it('disables issuance and revocation when the user directory fails', async () => {
  api.apiGet.mockImplementation((path) => path === '/api/users' ? Promise.reject(new Error('用户目录不可用')) : Promise.resolve({ tokens }));
  render(<TokensPanel />);
  await screen.findByText('用户目录不可用');
  expect(screen.getByRole('button', { name: '创建 Token' }).disabled).toBe(true);
  expect(screen.getByRole('button', { name: /撤\s*销/ }).disabled).toBe(true);
  expect(screen.getByText('CLI')).toBeTruthy();
  expect(screen.getByRole('button', { name: /重\s*试/ })).toBeTruthy();
});
it('explains the empty user directory and keeps administrator tokens protected', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/users'
    ? { users: [users[0]] } : { tokens: [{ ...tokens[0], user_name: 'admin' }] }));
  render(<TokensPanel />);
  await screen.findByText('请先在“用户权限”中创建用户，再为其签发 Token。');
  expect(screen.getByRole('button', { name: '创建 Token' }).disabled).toBe(true);
  expect(screen.queryByRole('button', { name: /撤\s*销/ })).toBeNull();
  expect(screen.getByText('启动配置管理')).toBeTruthy();
});
