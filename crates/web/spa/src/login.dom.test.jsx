// @vitest-environment jsdom
// LoginModal DOM 契约（token-only 登录）：
//   1. 不再有 服务器地址 输入 —— 地址只来自 URL/存储/VITE_OC_BASE 内嵌；
//   2. 提交探测 GET /api/me（成功 ⇒ setIdentity + onConnected，保留当前 base）；
//   3. 探测失败 ⇒ clearToken（oc_token 清空）+ 错误提示，base 原样保留。
// api.js 模块级 mock（同 agentsConfig.dom.test.jsx 模式）。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock } = vi.hoisted(() => ({ apiGetMock: vi.fn() }));
vi.mock('./api.js', () => ({ apiGet: apiGetMock }));
vi.mock('./sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

import './test/setup-dom.js';
import { LoginModal } from './login.jsx';
import { clearCredentials, getState, setCredentials } from './store.js';

const connect = () => fireEvent.click(screen.getByRole('button', { name: /连\s*接/ }));

beforeEach(() => {
  localStorage.clear();
  clearCredentials();
  apiGetMock.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('LoginModal (token-only)', () => {
  it('asks only for the token — the 服务器地址 field is gone', () => {
    render(<LoginModal open onConnected={() => {}} />);
    expect(screen.getByLabelText('访问令牌 (Token)')).toBeTruthy();
    expect(screen.queryByLabelText('服务器地址')).toBeNull();
  });

  it('probes /api/me, publishes the identity and keeps the current base', async () => {
    setCredentials('stale-token', 'https://fleet.example.com');
    apiGetMock.mockResolvedValueOnce({ name: 'ops', role: 'user' });
    const onConnected = vi.fn();
    render(<LoginModal open onConnected={onConnected} />);
    fireEvent.change(screen.getByLabelText('访问令牌 (Token)'), { target: { value: 'good-token' } });
    connect();
    await waitFor(() => expect(apiGetMock).toHaveBeenCalledWith('/api/me'));
    await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
    expect(getState().token).toBe('good-token');
    // base 仍是登录前的存储值（VITE_OC_BASE / URL 机制不动）。
    expect(getState().base).toBe('https://fleet.example.com');
    expect(getState().identity).toEqual({ name: 'ops', role: 'user' });
    expect(localStorage.getItem('oc_token')).toBe('good-token');
  });

  it('drops the token but keeps the base when the probe fails', async () => {
    setCredentials('', 'https://keep.example.com');
    apiGetMock.mockRejectedValueOnce(Object.assign(new Error('unauthorized'), { status: 401 }));
    render(<LoginModal open onConnected={() => {}} />);
    fireEvent.change(screen.getByLabelText('访问令牌 (Token)'), { target: { value: 'bad-token' } });
    connect();
    expect(await screen.findByText('连接失败: unauthorized')).toBeTruthy();
    await waitFor(() => expect(getState().token).toBe(''));
    await waitFor(() => expect(getState().identity).toBeNull());
    expect(localStorage.getItem('oc_token')).toBeNull();
    expect(getState().base).toBe('https://keep.example.com');
  });

  it('refuses an empty token without probing', async () => {
    render(<LoginModal open onConnected={() => {}} />);
    connect();
    expect(await screen.findByText('访问令牌不能为空')).toBeTruthy();
    expect(apiGetMock).not.toHaveBeenCalled();
  });
});
