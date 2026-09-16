// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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

it('creates a codex profile through the modal and selects it after the upsert', async () => {
  apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', revision: 7, settings: { envs: {} } }], profiles: [{ name: 'alpha', settings: { model: 'm1' } }] });
  apiPut.mockResolvedValue({ revision: 1 });
  render(<HarnessManagement onNotice={vi.fn()} />);
  fireEvent.click(await screen.findByRole('button', { name: '新建配置档案' }));
  const dialog = await screen.findByRole('dialog');
  fireEvent.change(within(dialog).getByLabelText('new-codex-profile'), { target: { value: 'beta' } });
  fireEvent.change(within(dialog).getByLabelText('new-codex-profile-model'), { target: { value: 'model-x' } });
  fireEvent.change(within(dialog).getByLabelText('new-codex-profile-envs'), { target: { value: 'A=1' } });
  fireEvent.click(within(dialog).getByRole('button', { name: /创\s*建/ }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/harnesses/codex/profiles/beta', { model: 'model-x', envs: { A: '1' } }));
  // 提交成功即关闭弹窗（jsdom 不跑离场动画，断言业务结果而非 DOM 摘除）。
  expect(await screen.findByText('配置档案 beta 已创建，新任务将使用此版本')).toBeTruthy();
  // 自动选中新档案：版本号与编辑表单都载入其 settings（DOM 顺序主表单在前）。
  expect(screen.getByText('配置 v1')).toBeTruthy();
  expect(screen.getAllByLabelText('模型（--model）')[0].value).toBe('model-x');
  expect(screen.getByLabelText('codex-managed-envs').value).toBe('A=1');
});

it('blocks invalid or duplicate profile names without submitting', async () => {
  apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', revision: 2, settings: { envs: {} } }], profiles: [{ name: 'alpha', settings: {} }] });
  render(<HarnessManagement onNotice={vi.fn()} />);
  fireEvent.click(await screen.findByRole('button', { name: '新建配置档案' }));
  const dialog = await screen.findByRole('dialog');
  const nameInput = within(dialog).getByLabelText('new-codex-profile');
  fireEvent.change(nameInput, { target: { value: '_bad' } });
  fireEvent.click(within(dialog).getByRole('button', { name: /创\s*建/ }));
  expect(await within(dialog).findByText('名称仅限字母、数字与 . _ -，以字母或数字开头，最长 48 字符')).toBeTruthy();
  fireEvent.change(nameInput, { target: { value: 'alpha' } });
  fireEvent.click(within(dialog).getByRole('button', { name: /创\s*建/ }));
  expect(await within(dialog).findByText('配置档案名称已存在')).toBeTruthy();
  expect(apiPut).not.toHaveBeenCalled();
});

it('saves node max concurrency and queue ordering together', async () => {
  apiGet.mockResolvedValue({ max_runs: 1, queue_order: 'lifo', workdir: '/data/old' });
  apiPut.mockResolvedValue({ max_runs: 3, queue_order: 'fifo', workdir: '/data/new' });
  const onSaved = vi.fn(); const onClose = vi.fn();
  render(<NodeSchedulingModal node={{ id: 'node-a', name: 'A', snapshot: { max_runs: 1, queue_order: 'lifo' } }} onSaved={onSaved} onClose={onClose} onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('node-workdir').value).toBe('/data/old'));
  fireEvent.change(screen.getByLabelText('node-max-runs'), { target: { value: '3' } });
  fireEvent.mouseDown(screen.getByLabelText('node-queue-order'));
  fireEvent.click(await waitFor(() => {
    const hit = [...document.querySelectorAll('.ant-select-item-option')].find((el) => el.title === '先入先出 FIFO');
    expect(hit).toBeTruthy(); return hit;
  }));
  fireEvent.change(screen.getByLabelText('node-workdir'), { target: { value: '/data/new' } });
  fireEvent.click(screen.getByRole('button', { name: '保存调度配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/nodes/node-a/scheduling', { max_runs: 3, queue_order: 'fifo', workdir: '/data/new' }));
  expect(onSaved).toHaveBeenCalled(); expect(onClose).toHaveBeenCalled();
});

it('submits null workdir when the node workspace input is cleared', async () => {
  apiGet.mockResolvedValue({ max_runs: 2, queue_order: 'fifo', workdir: null });
  apiPut.mockResolvedValue({ max_runs: 2, queue_order: 'fifo', workdir: null });
  const onSaved = vi.fn(); const onClose = vi.fn();
  render(<NodeSchedulingModal node={{ id: 'node-a', name: 'A', snapshot: { max_runs: 2, queue_order: 'fifo' } }} onSaved={onSaved} onClose={onClose} onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('node-workdir').value).toBe(''));
  fireEvent.change(screen.getByLabelText('node-workdir'), { target: { value: '' } });
  fireEvent.click(screen.getByRole('button', { name: '保存调度配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/nodes/node-a/scheduling', { max_runs: 2, queue_order: 'fifo', workdir: null }));
  expect(onSaved).toHaveBeenCalled(); expect(onClose).toHaveBeenCalled();
});

it('rejects a relative workdir without submitting', async () => {
  apiGet.mockResolvedValue({ max_runs: 2, queue_order: 'fifo', workdir: null });
  render(<NodeSchedulingModal node={{ id: 'node-a', name: 'A', snapshot: { max_runs: 2, queue_order: 'fifo' } }} onSaved={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('node-workdir').value).toBe(''));
  fireEvent.change(screen.getByLabelText('node-workdir'), { target: { value: 'rel/dir' } });
  fireEvent.click(screen.getByRole('button', { name: '保存调度配置' }));
  expect(await screen.findByText('工作空间必须是绝对路径')).toBeTruthy();
  expect(apiPut).not.toHaveBeenCalled();
});

it('multi-runtime hosts hide the workspace input and omit workdir from the payload', async () => {
  apiGet.mockResolvedValue({ max_runs: 2, queue_order: 'fifo', workdir: null, workdir_supported: false });
  apiPut.mockResolvedValue({ max_runs: 2, queue_order: 'fifo' });
  const onSaved = vi.fn(); const onClose = vi.fn();
  render(<NodeSchedulingModal node={{ id: 'node-host', name: 'H', snapshot: { max_runs: 2, queue_order: 'fifo' } }} onSaved={onSaved} onClose={onClose} onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('node-max-runs').value).toBe('2'));
  expect(screen.queryByLabelText('node-workdir')).toBeNull();
  expect(await screen.findByText('多运行时宿主不支持节点级工作空间，会话目录由各运行时自身决定。')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '保存调度配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/nodes/node-host/scheduling', { max_runs: 2, queue_order: 'fifo' }));
  expect(onSaved).toHaveBeenCalled(); expect(onClose).toHaveBeenCalled();
});
