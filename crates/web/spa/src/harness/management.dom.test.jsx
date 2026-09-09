// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { AgentHarnessSettings, HarnessManagement } from './management.jsx';
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
  fireEvent.change(screen.getByLabelText('Codex 模型'), { target: { value: 'model-b' } });
  fireEvent.change(screen.getByLabelText('codex-managed-envs'), { target: { value: 'KEY= literal = 中文\nEMPTY=' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 Codex 配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/harnesses/codex', {
    executable: '/usr/bin/codex', model: 'model-b', reasoning_effort: 'high', sandbox_mode: 'read-only', approval_policy: 'never', auth_slot: null, envs: { KEY: ' literal = 中文', EMPTY: '' },
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

it('builtin and custom agents both expose harness selection', async () => {
  apiGet.mockResolvedValue({ agents: [{ name: 'act', builtin: true, harness: 'opencoder' }, { name: 'reviewer', builtin: false, harness: 'codex' }] });
  apiPut.mockResolvedValue({ ok: true });
  render(<AgentHarnessSettings onNotice={vi.fn()} />);
  fireEvent.mouseDown(await screen.findByLabelText('harness-act'));
  const option = await waitFor(() => {
    const element = document.querySelector('.ant-select-dropdown .ant-select-item-option[title="Codex"]');
    expect(element).toBeTruthy(); return element;
  });
  fireEvent.click(option);
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/agents/act', { harness: 'codex' }));
  expect(screen.getByLabelText('harness-reviewer')).toBeTruthy();
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
