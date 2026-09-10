// @vitest-environment jsdom
// Operator 页签可见性 + 面板内容：AgentsPanel 仅在 identity.role === 'admin'
// 时渲染 Operator tab；面板说明、节点表（kinds 含 operator 才可启动）都在。
// store identity 经 setState 直写（同 app.dom.test.jsx 的 store 驱动方式）。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPatchMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPatchMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPatch: apiPatchMock,
  apiDel: apiDelMock,
}));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

import '../test/setup-dom.js';
import { AgentsPanel } from '../agentsConfig.jsx';
import { clearCredentials, setState } from '../store.js';

const nodesFixture = {
  nodes: [
    { id: 'node-1', name: 'edge-1', online: true, snapshot: { ready: true, cpu_capacity: 16, active_agent_loops: 2, pending_runs: 0 }, kinds: ['agent', 'operator'] },
    { id: 'node-2', name: 'edge-2', online: true, snapshot: { ready: true }, kinds: ['agent'] },
    { id: 'node-3', name: 'edge-3', online: false, snapshot: null, kinds: ['operator'] },
  ],
};

beforeEach(() => {
  clearCredentials();
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/nodes') return Promise.resolve(nodesFixture);
    if (path === '/api/agents') return Promise.resolve({ agents: [{ name: 'act' }] });
    return Promise.resolve({ ok: true, resources: [] });
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true });
  apiPatchMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
});

afterEach(() => {
  cleanup();
  clearCredentials();
});

describe('Operator tab visibility', () => {
  it('shows the Operator tab for an admin identity and renders its panel', async () => {
    setState({ identity: { name: 'boss', role: 'admin' } });
    render(<AgentsPanel onNotice={() => {}} />);
    expect(screen.getByText('Agent 列表')).toBeTruthy();
    fireEvent.click(screen.getByText('Operator'));
    // 面板说明（非 runc 容器、非节点维护模式）+ 节点表。
    expect(await screen.findByText(/非 runc 容器、非节点维护模式/)).toBeTruthy();
    expect(await screen.findByText('edge-1')).toBeTruthy();
    // 启动按钮：node-1 可用；node-2 缺 operator kind、node-3 离线 ⇒ 禁用。
    const texts = await screen.findAllByText('启动 Operator');
    const launchButtons = texts.map((el) => el.closest('button'));
    expect(launchButtons).toHaveLength(3);
    expect(launchButtons[0].disabled).toBe(false);
    expect(launchButtons[1].disabled).toBe(true);
    expect(launchButtons[2].disabled).toBe(true);
  });

  it('hides the Operator tab for a non-admin identity', async () => {
    setState({ identity: { name: 'guest', role: 'user' } });
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findByText('Agent 列表');
    expect(screen.queryByText('Operator')).toBeNull();
  });
});

describe('OperatorPanel launch flow', () => {
  it('opens the launch modal from an operable node', async () => {
    setState({ identity: { name: 'boss', role: 'admin' } });
    render(<AgentsPanel onNotice={() => {}} />);
    fireEvent.click(screen.getByText('Operator'));
    const launchButtons = (await screen.findAllByText('启动 Operator')).map((el) => el.closest('button'));
    fireEvent.click(launchButtons[0]);
    expect(await screen.findByText('启动 Operator · edge-1')).toBeTruthy();
  });
});
