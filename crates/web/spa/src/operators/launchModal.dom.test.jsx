// @vitest-environment jsdom
// LaunchModal DOM 契约：agent 选项来自 GET /api/agents（默认 act），提交构造
// operator 执行载荷（kind/target/node_id/input.prompt/input.harness，id 前缀
// operator-），受理后 onClose + onAccepted(accepted)。api.js 模块级 mock。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock } = vi.hoisted(() => ({ apiGetMock: vi.fn(), apiPostMock: vi.fn() }));
vi.mock('../api.js', () => ({ apiGet: apiGetMock, apiPost: apiPostMock }));

import '../test/setup-dom.js';
import { LaunchModal } from './launchModal.jsx';

const node = { id: 'node-7', name: 'edge-7', online: true, snapshot: { ready: true }, kinds: ['agent', 'operator'] };
const accepted = { id: 'operator-abc', kind: 'operator', node_id: 'node-7', status: 'pending' };

beforeEach(() => {
  apiGetMock.mockReset().mockResolvedValue({ agents: [{ name: 'act' }, { name: 'coder' }] });
  apiPostMock.mockReset().mockResolvedValue(accepted);
});

afterEach(() => {
  cleanup();
});

describe('LaunchModal', () => {
  it('submits an operator execution payload bound to the node', async () => {
    const onClose = vi.fn();
    const onAccepted = vi.fn();
    render(<LaunchModal node={node} onClose={onClose} onAccepted={onAccepted} onNotice={() => {}} />);
    expect(apiGetMock).toHaveBeenCalledWith('/api/agents');
    // 只读节点字段带出节点标识。
    expect(screen.getByDisplayValue('edge-7（node-7）')).toBeTruthy();
    fireEvent.change(screen.getByLabelText('任务要求'), { target: { value: '巡检宿主机磁盘水位' } });
    fireEvent.click(screen.getByRole('button', { name: /启动并查看/ }));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(1));
    const [path, body] = apiPostMock.mock.calls[0];
    expect(path).toBe('/api/executions');
    expect(body).toMatchObject({
      kind: 'operator',
      target: 'act',
      node_id: 'node-7',
      input: { prompt: '巡检宿主机磁盘水位', harness: 'opencoder' },
    });
    expect(body.id).toMatch(/^operator-[0-9a-f]{32}$/);
    expect(onClose).toHaveBeenCalled();
    expect(onAccepted).toHaveBeenCalledWith(accepted);
  });

  it('keeps the same execution id for an identical retry (idempotency attempt)', async () => {
    apiPostMock.mockRejectedValueOnce(new Error('节点确认超时'));
    const onNotice = vi.fn();
    render(<LaunchModal node={node} onClose={() => {}} onAccepted={() => {}} onNotice={onNotice} />);
    fireEvent.change(screen.getByLabelText('任务要求'), { target: { value: '重试同一任务' } });
    fireEvent.click(screen.getByRole('button', { name: /启动并查看/ }));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(1));
    const firstId = apiPostMock.mock.calls[0][1].id;
    fireEvent.click(screen.getByRole('button', { name: /启动并查看/ }));
    await waitFor(() => expect(apiPostMock).toHaveBeenCalledTimes(2));
    expect(apiPostMock.mock.calls[1][1].id).toBe(firstId);
    expect(onNotice).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
  });
});
