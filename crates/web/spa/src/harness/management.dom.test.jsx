// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { HarnessManagement } from './management.jsx';
import { NodeSchedulingModal } from '../fleet/settings/scheduling.jsx';
import { apiGet, apiPut } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPut: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it('edits managed Codex parameters and literal env as one persisted configuration', async () => {
  apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', managed: true, revision: 3, settings: {
    executable: '/usr/bin/codex', model: 'model-a', reasoning_effort: 'high', sandbox_mode: 'read-only', approval_policy: 'never', envs: { OLD: 'value' },
  } }] });
  apiPut.mockResolvedValue({ revision: 4 });
  render(<HarnessManagement onNotice={vi.fn()} />);
  expect(await screen.findByText('配置 v3')).toBeTruthy();
  for (const label of ['Codex 二进制路径', '授权槽位', '推理强度', '沙箱权限', '审批策略']) expect(screen.queryByLabelText(label)).toBeNull();
  fireEvent.change(screen.getByLabelText('模型（--model）'), { target: { value: 'model-b' } });
  fireEvent.change(screen.getByLabelText('codex-managed-envs'), { target: { value: 'KEY= literal = 中文\nEMPTY=' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 Codex 配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/harnesses/codex', {
    model: 'model-b', envs: { KEY: ' literal = 中文', EMPTY: '' },
  }));
  expect(await screen.findByText('配置 v4')).toBeTruthy();
});

it('invalid managed env prevents saving and load errors never submit empty defaults', async () => {
  apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', settings: { envs: {} } }] });
  render(<HarnessManagement onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByRole('button', { name: '保存 Codex 配置' }).disabled).toBe(false));
  fireEvent.change(screen.getByLabelText('codex-managed-envs'), { target: { value: 'INVALID' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 Codex 配置' }));
  expect(await screen.findByText('环境变量必须为 KEY=VALUE，每行一个')).toBeTruthy();
  expect(apiPut).not.toHaveBeenCalled();
  cleanup();
  apiGet.mockRejectedValue(new Error('unavailable'));
  const notice = vi.fn(); render(<HarnessManagement onNotice={notice} />);
  await waitFor(() => expect(notice).toHaveBeenCalled());
  expect(screen.getByRole('button', { name: '保存 Codex 配置' }).disabled).toBe(true);
});

it('keeps model and environment edits after a failed save so the same configuration can be retried', async () => {
  apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', revision: 1, settings: { envs: {} } }], profiles: [] });
  apiPut.mockRejectedValueOnce(new Error('unavailable')).mockResolvedValueOnce({ revision: 2 });
  const notice = vi.fn();
  render(<HarnessManagement onNotice={notice} />);
  await screen.findByText('配置 v1');
  fireEvent.change(screen.getByLabelText('模型（--model）'), { target: { value: 'model-retry' } });
  fireEvent.change(screen.getByLabelText('codex-managed-envs'), { target: { value: 'KEY=literal=value' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 Codex 配置' }));
  await waitFor(() => expect(notice).toHaveBeenCalledWith({ type: 'error', text: '保存 Harness 配置失败：unavailable' }));
  expect(screen.getByLabelText('模型（--model）').value).toBe('model-retry');
  expect(screen.getByLabelText('codex-managed-envs').value).toBe('KEY=literal=value');
  expect(screen.getByLabelText('codex-profile').disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '保存 Codex 配置' }));
  await screen.findByText('配置 v2');
  expect(apiPut.mock.calls).toEqual(Array(2).fill(['/api/harnesses/codex', { model: 'model-retry', envs: { KEY: 'literal=value' } }]));
});

it('saves node max concurrency and queue ordering together', async () => {
  apiPut.mockResolvedValue({ max_runs: 3, queue_order: 'lifo' });
  const onSaved = vi.fn(); const onClose = vi.fn();
  render(<NodeSchedulingModal node={{ id: 'node-a', name: 'A', snapshot: { max_runs: 1, queue_order: 'lifo' } }} onSaved={onSaved} onClose={onClose} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('node-max-runs'), { target: { value: '3' } });
  fireEvent.click(screen.getByRole('button', { name: '保存调度配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/nodes/node-a/scheduling', { max_runs: 3, queue_order: 'lifo' }));
  expect(onSaved).toHaveBeenCalled(); expect(onClose).toHaveBeenCalled();
});
