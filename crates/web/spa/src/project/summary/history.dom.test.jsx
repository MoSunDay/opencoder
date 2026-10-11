// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
import { apiGet, apiPost } from '../../api.js';
import { setIdentity } from '../../store.js';
import { CurrentSummary } from './view.jsx';

const versions = [
  { version: 3, current_summary: '最新概述', created_at: 1700000003000, restored_from: 1 },
  { version: 2, current_summary: '可恢复的概述', created_at: 1700000002000 },
  { version: 1, current_summary: '', created_at: 1700000001000 },
];
const props = { collection: 'goals', record: { id: 'p', current_summary: '最新概述' } };
beforeEach(() => {
  vi.resetAllMocks(); setIdentity({ name: 'editor', role: 'editor' });
  apiGet.mockResolvedValue({ versions, limit: 10 }); apiPost.mockResolvedValue({ version: 4 });
});

it('reads history on demand and restores a selected version after confirmation', async () => {
  const refresh = vi.fn(async () => {});
  render(<CurrentSummary {...props} refresh={refresh} />);
  expect(apiGet).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '修改记录' }));
  expect(await screen.findByText('可恢复的概述')).toBeTruthy();
  expect(screen.getByText('版本 3（当前）')).toBeTruthy();
  expect(screen.getByText('恢复自版本 1')).toBeTruthy();
  expect(screen.getByText('空概述')).toBeTruthy();
  expect(apiGet.mock.calls[0][0]).toBe('/api/project/goals/p/summary-versions');
  fireEvent.click(screen.getAllByRole('button', { name: '恢复此版本' })[0]);
  expect(apiPost).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByRole('button', { name: '确认恢复' }));
  await waitFor(() => expect(refresh).toHaveBeenCalledOnce());
  expect(apiPost.mock.calls[0][0]).toBe('/api/project/goals/p/summary-versions/2/restore');
  await waitFor(() => expect(apiGet).toHaveBeenCalledTimes(2));
});

it('shows read failures with retry and retains history after a restore failure', async () => {
  apiGet.mockRejectedValueOnce(new Error('记录暂时不可用'));
  apiPost.mockRejectedValue(new Error('该版本已被清理'));
  const refresh = vi.fn();
  render(<CurrentSummary {...props} collection="initiatives" refresh={refresh} />);
  fireEvent.click(screen.getByRole('button', { name: '修改记录' }));
  expect(await screen.findByText('记录暂时不可用')).toBeTruthy();
  expect(screen.queryByText('暂无修改记录')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
  await screen.findByText('可恢复的概述');
  fireEvent.click(screen.getAllByRole('button', { name: '恢复此版本' })[0]);
  fireEvent.click(await screen.findByRole('button', { name: '确认恢复' }));
  expect(await screen.findByText('该版本已被清理')).toBeTruthy();
  expect(screen.getByText('可恢复的概述')).toBeTruthy();
  expect(refresh).not.toHaveBeenCalled();
  expect(apiPost.mock.calls[0][0]).toBe('/api/project/initiatives/p/summary-versions/2/restore');
});

it('allows viewers to read history but disables restoration', async () => {
  setIdentity({ name: 'viewer', role: 'viewer' });
  render(<CurrentSummary {...props} />);
  fireEvent.click(screen.getByRole('button', { name: '修改记录' }));
  await screen.findByText('可恢复的概述');
  for (const button of screen.getAllByRole('button', { name: '恢复此版本' })) expect(button.disabled).toBe(true);
  expect(apiPost).not.toHaveBeenCalled();
});

it('discards a history response after switching to another record', async () => {
  let resolveOld;
  apiGet.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
  const view = render(<CurrentSummary {...props} />);
  fireEvent.click(screen.getByRole('button', { name: '修改记录' }));
  await waitFor(() => expect(apiGet).toHaveBeenCalledOnce());
  const signal = apiGet.mock.calls[0][1].signal;
  view.rerender(<CurrentSummary {...props} record={{ id: 'other', current_summary: '其他项目概述' }} />);
  expect(signal.aborted).toBe(true);
  resolveOld({ versions, limit: 10 });
  fireEvent.click(screen.getByRole('button', { name: '修改记录' }));
  await screen.findByText('可恢复的概述');
  expect(apiGet.mock.calls.at(-1)[0]).toBe('/api/project/goals/other/summary-versions');
});
