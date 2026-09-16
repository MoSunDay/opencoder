// @vitest-environment jsdom
// Operator 页签可见性 + 面板内容：AgentsPanel 仅在 identity.role === 'admin'
// 时渲染 Operator tab；面板为只读节点总览（会话交互页以 operator kind 建会话，
// 本页不再有「操作」列/启动入口）。store identity 经 setState 直写。

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
    // 面板说明（会话即 Operator 运行：非 runc 容器、非节点维护模式）+ 节点表。
    expect(await screen.findByText(/非 runc 容器、非节点维护模式/)).toBeTruthy();
    expect(await screen.findByText('edge-1')).toBeTruthy();
    expect(await screen.findByText('edge-3')).toBeTruthy();
  });

  it('keeps the panel read-only: no actions column or launch entry', async () => {
    setState({ identity: { name: 'boss', role: 'admin' } });
    render(<AgentsPanel onNotice={() => {}} />);
    fireEvent.click(screen.getByText('Operator'));
    expect(await screen.findByText('edge-1')).toBeTruthy();
    expect(screen.queryByText('操作')).toBeNull();
    expect(screen.queryByText('启动 Operator')).toBeNull();
    expect(screen.queryByRole('button', { name: /启动/ })).toBeNull();
  });

  it('hides the Operator tab for a non-admin identity', async () => {
    setState({ identity: { name: 'guest', role: 'user' } });
    render(<AgentsPanel onNotice={() => {}} />);
    await screen.findByText('Agent 列表');
    expect(screen.queryByText('Operator')).toBeNull();
  });
});
