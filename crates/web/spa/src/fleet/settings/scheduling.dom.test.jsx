// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import '../../test/setup-dom.js';
const mocks = vi.hoisted(() => ({ apiGet: vi.fn(), apiPut: vi.fn() }));
vi.mock('../../api.js', () => mocks);
import { NodeSchedulingModal } from './scheduling.jsx';
beforeEach(() => { mocks.apiGet.mockReset(); mocks.apiPut.mockReset().mockResolvedValue({ ok: true }); });

it('does not save invented settings when the node configuration cannot be read', async () => {
  mocks.apiGet.mockRejectedValueOnce(new Error('node offline')).mockResolvedValue({ max_runs: 3, queue_order: 'fifo', workdir_supported: false });
  render(<NodeSchedulingModal node={{ id: 'node-a', name: 'worker', snapshot: { max_runs: 99 } }} onClose={vi.fn()} onSaved={vi.fn()} onNotice={vi.fn()} />);
  expect(await screen.findByText('读取调度配置失败：node offline')).toBeTruthy();
  expect(screen.queryByRole('button', { name: '保存调度配置' })).toBeNull();
  expect(mocks.apiPut).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '重试读取配置' }));
  await screen.findByRole('button', { name: '保存调度配置' });
  expect(screen.queryByLabelText('node-workdir')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '保存调度配置' }));
  await waitFor(() => expect(mocks.apiPut).toHaveBeenCalledWith('/api/nodes/node-a/scheduling', { max_runs: 3, queue_order: 'fifo' }));
});
