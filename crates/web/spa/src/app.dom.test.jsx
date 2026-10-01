// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { App, LoginModal, bootUrlCredential, clearCredentials, embeddedBase, getState, setCredentials, setState, HEADERLESS_REASONS, NAV_CATEGORIES, ConfigProvider, Popconfirm, zhCN, dayjs, theme, installFetchRouter, jsonResponse, deprecationHits, mountApp } from './test/appShell.jsx';

describe('App shell landmarks (antd 6 under jsdom)', () => {
  it('gates unauthenticated visitors behind the login modal', async () => {
    await mountApp();
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    // Nothing renders behind the gate: no fleet table without a token.
    expect(screen.queryByText('暂无 Opencoder 节点')).toBeNull();
  });

  it('notifies the app to clear stale request errors after a successful login', async () => {
    const onConnected = vi.fn();
    render(<LoginModal open onConnected={onConnected} />);
    fireEvent.change(screen.getByLabelText('访问令牌 (Token)'), {
      target: { value: 'correct-token' },
    });
    fireEvent.click(screen.getByRole('button', { name: /连\s*接/ }));
    await waitFor(() => expect(onConnected).toHaveBeenCalledOnce());
    expect(localStorage.getItem('oc_token')).toBe('correct-token');
  });

  it('logs in from a ?token= link, scrubbing the secret from the URL', async () => {
    window.history.replaceState(null, '', '/?view=nodes&token=url-secret#fleet');
    bootUrlCredential();
    await mountApp();
    // No login modal: the fleet table renders straight away.
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(window.location.search).toBe('?view=nodes');
    expect(window.location.hash).toBe('#fleet');
    expect(localStorage.getItem('oc_token')).toBe('url-secret');
  });

  it('logs in from a #token= fragment link and scrubs it', async () => {
    window.history.replaceState(null, '', '/#token=frag-secret');
    bootUrlCredential();
    await mountApp();
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(window.location.hash).toBe('');
    expect(localStorage.getItem('oc_token')).toBe('frag-secret');
  });

  it('a URL token overrides stored credentials for authenticated visitors', async () => {
    // The race this case pins down: the stale token must never win — boot
    // adopts the URL token before any component can fire a 401-ing request.
    setCredentials('stored-token', '');
    window.history.replaceState(null, '', '/?token=url-secret#fleet');
    bootUrlCredential();
    await mountApp();
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(window.location.search).toBe('');
    expect(localStorage.getItem('oc_token')).toBe('url-secret');
  });

  it('falls back to the login modal when a URL token is rejected (401)', async () => {
    installFetchRouter({ rejectToken: 'bad-secret' });
    window.history.replaceState(null, '', '/?token=bad-secret');
    bootUrlCredential();
    await mountApp();
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    expect(window.location.search).toBe('');
    expect(localStorage.getItem('oc_token')).toBeNull();
  });

  it('keeps the URL-delivered base when the URL token is rejected (401)', async () => {
    installFetchRouter({ rejectToken: 'bad-secret' });
    window.history.replaceState(null, '', '/#token=bad-secret&base=http://fleet2.example.com');
    bootUrlCredential();
    await mountApp();
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    // The 401 cleared the token but must not wipe the link-delivered base:
    // login is token-only now (no 服务器地址 field), so the surviving base is
    // the store/localStorage one the next probe reuses.
    expect(screen.queryByLabelText('服务器地址')).toBeNull();
    expect(getState().base).toBe('http://fleet2.example.com');
    expect(localStorage.getItem('oc_base')).toBe('http://fleet2.example.com');
  });

  it('adopts a base-only link: base stored, session token kept, url scrubbed', () => {
    setCredentials('stored-token', '');
    window.history.replaceState(null, '', '/#base=http://fleet2.example.com');
    bootUrlCredential();
    expect(window.location.hash).toBe('');
    expect(localStorage.getItem('oc_token')).toBe('stored-token');
    expect(localStorage.getItem('oc_base')).toBe('http://fleet2.example.com');
    expect(getState().base).toBe('http://fleet2.example.com');
  });

  it('embeddedBase() honors a baked VITE_OC_BASE (trimmed, slash-stripped)', () => {
    vi.stubEnv('VITE_OC_BASE', '  https://fleet.example.com/  ');
    expect(embeddedBase()).toBe('https://fleet.example.com');
    vi.stubEnv('VITE_OC_BASE', '');
    expect(embeddedBase()).toBe('');
  });

  it('re-probes /api/me on refresh and restores the admin identity', async () => {
    // A page reload (or a ?token= link login) restores the token but not the
    // identity — the store never persists /api/me. The shell must probe on
    // mount so the badge, admin entry, and admin nav come back by themselves.
    setCredentials('smoke-token', '');
    await mountApp();
    expect(await screen.findByText('smoke · 管理员')).toBeTruthy();
    expect(await screen.findByRole('button', { name: '后台管理' })).toBeTruthy();
    expect(getState().identity).toEqual({ name: 'smoke', role: 'admin' });
    // No login modal: the stored credential was accepted.
    expect(screen.queryByText('Opencoder Fleet · 登录')).toBeNull();
  });

  it('drops a stored token the refresh probe rejects (401) back to the login modal', async () => {
    installFetchRouter({ rejectToken: 'stale-token' });
    setCredentials('stale-token', '');
    await mountApp();
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
    expect(localStorage.getItem('oc_token')).toBeNull();
    expect(getState().identity).toBeNull();
  });

  it('shows the empty fleet table on the nodes page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'nodes' });
    await mountApp();
    // findBy*: the table fills in only after the mocked /api/nodes round-trip.
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
  });

  it('surfaces panel errors as a closable error Alert notice', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'nodes' });
    installFetchRouter({ failNodes: true });
    await mountApp();
    // The nodes panel routes its load failure through onNotice, which the
    // shell now renders as an antd Alert (role=alert) instead of red text.
    const alert = (await screen.findByText('节点服务不可用')).closest('[role="alert"]');
    expect(alert.textContent).toContain('节点服务不可用');
    // R1: the shell renders the notice's own severity — a load failure stays
    // a red error Alert (ant-alert-error), not a generic one.
    expect(alert.className).toContain('ant-alert-error');
    // Closable: the close button carries an explicit aria-label, and closing
    // it clears the notice (asserted before the 3s poll can re-arm it).
    fireEvent.click(within(alert).getByRole('button', { name: '关闭' }));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('renders a success Alert after a real goal-create round-trip on the project page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'project' });
    installFetchRouter({ withGoal: true });
    await mountApp();
    // Real interaction flow: open the goals tab, 新建目标 modal, fill the
    // form, submit — the fetch router answers 200 so GoalsTab reports
    // ok('目标已创建') and the shell paints it green (R1 fix).
    fireEvent.click(await screen.findByRole('tab', { name: '项目', selected: true }));
    await act(async () => { await new Promise((r) => setTimeout(r, 20)); });
    fireEvent.click(await screen.findByText('新建项目'));
    await act(async () => { await new Promise((r) => setTimeout(r, 20)); });
    fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: '新目标' } });
    // antd auto-inserts a space between the CJK glyphs (保 存) — strip it.
    const save = [...document.querySelectorAll('button')].find((b) => b.textContent.replace(/\s+/g, '') === '保存');
    expect(save).toBeTruthy();
    fireEvent.click(save);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('项目已保存');
    expect(alert.className).toContain('ant-alert-success');
    expect(alert.className).not.toContain('ant-alert-error');
  });

  it('logs out from the Header: token cleared, login gate reopens', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    // The header's right side carries the same-origin readout + 退出.
    expect(screen.getByText('同源')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '退出' }));
    // clearCredentials(): persisted token gone, state token '' → gate reopens.
    expect(localStorage.getItem('oc_token')).toBeNull();
    expect(await screen.findByText('Opencoder Fleet · 登录')).toBeTruthy();
  });


  it('does not render privileged navigation before identity confirmation', async () => {
    let reply;
    vi.stubGlobal('fetch', vi.fn(() => new Promise((resolve) => { reply = resolve; })));
    setCredentials('waiting-token', '');
    render(<App />);
    expect(screen.getByText('正在确认登录身份…')).toBeTruthy();
    expect(screen.queryByRole('menuitem')).toBeNull();
    expect(screen.queryByRole('button', { name: '后台管理' })).toBeNull();
    await act(async () => { reply({ ok: true, status: 200, json: async () => ({ name: 'reader', role: 'user' }) }); });
    expect(await screen.findByRole('menuitem', { name: /全部执行/ })).toBeTruthy();
    expect(screen.queryByRole('menuitem', { name: /节点列表/ })).toBeNull();
  });

  it('keeps identity failures explicit and retries without showing privileged pages', async () => {
    vi.stubGlobal('fetch', vi.fn(() => jsonResponse({ error: 'identity unavailable' }, 503)));
    setCredentials('waiting-token', '');
    render(<App />);
    expect(await screen.findByText('身份确认失败：identity unavailable')).toBeTruthy();
    expect(screen.queryByRole('menuitem')).toBeNull();
    installFetchRouter();
    fireEvent.click(screen.getByRole('button', { name: '重试身份确认' }));
    expect(await screen.findByRole('menuitem', { name: /节点列表/ })).toBeTruthy();
  });

  it('rejects malformed identity responses rather than waiting forever', async () => {
    vi.stubGlobal('fetch', vi.fn(() => jsonResponse({})));
    setCredentials('waiting-token', '');
    render(<App />);
    expect(await screen.findByText('身份确认失败：登录身份响应格式错误，请重试')).toBeTruthy();
    expect(screen.queryByRole('menuitem')).toBeNull();
  });
});
