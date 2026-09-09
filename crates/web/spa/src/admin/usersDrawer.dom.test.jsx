// @vitest-environment jsdom
// UsersDrawer DOM 契约：用户表渲染 fixture（角色/创建时间/吊销）、创建表单
// POST /api/users {name, role}、令牌只在一次性 Modal 里出现、吊销走
// Popconfirm 确认后命中 DELETE /api/users/:name。api.js 模块级 mock。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('../api.js', () => ({ apiGet: apiGetMock, apiPost: apiPostMock, apiDel: apiDelMock }));

import '../test/setup-dom.js';
import { UsersDrawer } from './usersDrawer.jsx';

const usersFixture = {
  users: [
    { name: 'root', role: 'root', created_at: '2026-09-01T00:00:00Z' },
    { name: 'ops-admin', role: 'admin', created_at: '2026-09-02T00:00:00Z' },
  ],
};

/// antd 6 Button 对两字中文自动插空格（「创 建」），按 role + 去空白匹配。
const findButton = (txt) => screen.getByRole('button', {
  name: (name) => name.replace(/\s+/g, '') === txt,
});

beforeEach(() => {
  apiGetMock.mockReset().mockResolvedValue(usersFixture);
  apiPostMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
});

afterEach(() => {
  cleanup();
});

describe('UsersDrawer', () => {
  it('loads and renders the user rows', async () => {
    render(<UsersDrawer open onClose={() => {}} onNotice={() => {}} />);
    expect(apiGetMock).toHaveBeenCalledWith('/api/users');
    expect(await screen.findByText('ops-admin')).toBeTruthy();
    expect(screen.getByText('root')).toBeTruthy();
    expect(screen.getByText('管理员')).toBeTruthy();
    expect(screen.getByText('Root')).toBeTruthy();
  });

  it('creates a user and shows the one-time token modal', async () => {
    apiPostMock.mockResolvedValueOnce({
      user: { name: 'newbie', role: 'user', created_at: '2026-09-09T00:00:00Z' },
      token: 'oc-once-only-token',
    });
    render(<UsersDrawer open onClose={() => {}} onNotice={() => {}} />);
    await screen.findByText('ops-admin');
    fireEvent.change(screen.getByPlaceholderText('用户名'), { target: { value: 'newbie' } });
    await act(async () => {
      fireEvent.click(findButton('创建用户'));
    });
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledWith('/api/users', { name: 'newbie', role: 'user' }));
    // 令牌只显示一次：一次性 Modal 承载 token 文案。
    expect(await screen.findByText('oc-once-only-token')).toBeTruthy();
    // 创建成功后重载列表。
    await waitFor(() => expect(apiGetMock.mock.calls.filter(([p]) => p === '/api/users').length).toBeGreaterThanOrEqual(2));
  });

  it('revokes a user via Popconfirm → DELETE /api/users/:name', async () => {
    render(<UsersDrawer open onClose={() => {}} onNotice={() => {}} />);
    await screen.findByText('ops-admin');
    const revokeInRow = within(screen.getByText('ops-admin').closest('tr')).getAllByRole('button', {
      name: (name) => name.replace(/\s+/g, '') === '吊销',
    })[0];
    await act(async () => {
      fireEvent.click(revokeInRow);
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /确认吊销/ }));
    });
    await waitFor(() => expect(apiDelMock).toHaveBeenCalledWith('/api/users/ops-admin'));
  });

  it('surfaces server errors (last admin / self-delete) through onNotice', async () => {
    apiDelMock.mockRejectedValueOnce(Object.assign(new Error('不能吊销最后一个管理员'), { status: 400 }));
    const onNotice = vi.fn();
    render(<UsersDrawer open onClose={() => {}} onNotice={onNotice} />);
    await screen.findByText('ops-admin');
    const revokeInRow = within(screen.getByText('ops-admin').closest('tr')).getAllByRole('button', {
      name: (name) => name.replace(/\s+/g, '') === '吊销',
    })[0];
    await act(async () => {
      fireEvent.click(revokeInRow);
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /确认吊销/ }));
    });
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(expect.objectContaining({ type: 'error', text: '不能吊销最后一个管理员' })));
  });
});
