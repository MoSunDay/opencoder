// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { ExecutionsPanel } from '../fleet/executions.jsx';
import { apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../fleet/detail.jsx', () => ({ ExecutionDetail: () => null }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it('launches Codex using managed parameters without per-run environment overrides', async () => {
  apiGet.mockResolvedValue({ executions: [], nodes: [] });
  apiPost.mockResolvedValue({ id: 'accepted' });
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  fireEvent.change(screen.getByPlaceholderText('act / 定义名称 / 任务 ID'), { target: { value: 'act' } });
  fireEvent.change(screen.getByLabelText('任务要求'), { target: { value: 'read files' } });
  fireEvent.mouseDown(screen.getByLabelText('agent-harness'));
  fireEvent.click(await screen.findByText('Codex'));
  expect(await screen.findByText('Codex 参数已统一管理')).toBeTruthy();
  expect(screen.queryByLabelText('agent-envs')).toBeNull();
  fireEvent.click(screen.getByText('启动执行'));
  await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/executions', expect.objectContaining({
    kind: 'agent', target: 'act', input: { prompt: 'read files', harness: 'codex', envs: {} },
  })));
});
