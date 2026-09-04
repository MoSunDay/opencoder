// @vitest-environment jsdom
// AgentNfsCard DOM smoke: GET /api/agents/nfs 的状态字段渲染（运行中 tag /
// host:port / 只读 / 导出根 / mount 提示行），Switch 翻转命中 POST
// /api/agents/nfs 且 body 带布尔 enabled。api.js 模块级 mock（同 envsPanel
// 模式）。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
}));
vi.mock('./api.js', () => ({ apiGet: apiGetMock, apiPost: apiPostMock }));

import './test/setup-dom.js';
import { AgentNfsCard } from './agentNfsCard.jsx';

const statusFixture = {
  running: true,
  host: '127.0.0.1',
  port: 2049,
  read_only: true,
  export_root: '/root/.opencoder/agents',
};

beforeEach(() => {
  apiGetMock.mockReset().mockResolvedValue({ ok: true, status: statusFixture });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, status: { ...statusFixture, running: false }, started: false });
});

afterEach(() => {
  cleanup();
});

describe('AgentNfsCard', () => {
  it('renders the status snapshot and the mount hint while running', async () => {
    render(<AgentNfsCard onNotice={() => {}} />);
    expect(await screen.findByText('运行中')).toBeTruthy();
    expect(screen.getByText('127.0.0.1:2049')).toBeTruthy();
    expect(screen.getByText('/root/.opencoder/agents')).toBeTruthy();
    expect(screen.getByLabelText('nfs-mount-hint').textContent)
      .toBe('mount -t nfs -o vers=3,tcp,port=2049,mountport=2049,nolock 127.0.0.1:/ <dir>');
  });

  it('flips the switch through POST /api/agents/nfs with enabled', async () => {
    render(<AgentNfsCard onNotice={() => {}} />);
    expect(await screen.findByText('运行中')).toBeTruthy();
    fireEvent.click(screen.getByRole('switch'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/agents/nfs', { enabled: false });
    });
    // POST 响应里的新状态接管渲染（停止后不再显示挂载提示）。
    await waitFor(() => {
      expect(screen.getByText('已停止')).toBeTruthy();
    });
    expect(screen.queryByLabelText('nfs-mount-hint')).toBeNull();
  });

  it('shows the stopped view and POSTs enabled:true when off', async () => {
    apiGetMock.mockReset().mockResolvedValue({
      ok: true,
      status: { running: false, host: '', port: 0, read_only: true, export_root: '' },
    });
    apiPostMock.mockReset().mockResolvedValue({
      ok: true,
      status: statusFixture,
      started: true,
    });
    render(<AgentNfsCard onNotice={() => {}} />);
    expect(await screen.findByText('已停止')).toBeTruthy();
    fireEvent.click(screen.getByRole('switch'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/agents/nfs', { enabled: true });
    });
  });
});
