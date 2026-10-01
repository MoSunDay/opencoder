// @vitest-environment jsdom
import '../test/setup-dom.js';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ScheduleRunsDrawer } from './history.jsx';

const { apiGet } = vi.hoisted(() => ({ apiGet: vi.fn() }));
vi.mock('../api.js', () => ({ apiGet }));
vi.mock('../fleet/detail.jsx', () => ({ ExecutionDetail: ({ id, onClose }) => <button onClick={onClose}>execution detail {id}</button> }));
afterEach(() => { cleanup(); apiGet.mockReset(); });

it('opens the recorded execution rather than launching another task', async () => {
  apiGet.mockResolvedValue({ runs: [{ scheduled_for_ms: 1000, fired_at_ms: 1000, status: 'fired', execution_id: 'saved-run' }] });
  render(<ScheduleRunsDrawer schedule={{ id: 'schedule-a' }} onClose={() => {}} onNotice={() => {}} />);
  fireEvent.click(await screen.findByRole('button', { name: 'saved-run' }));
  expect(screen.getByRole('button', { name: 'execution detail saved-run' })).toBeTruthy();
  expect(apiGet.mock.calls).toHaveLength(1);
});

it('keeps a failed history read distinct from no triggers and allows retry', async () => {
  apiGet.mockRejectedValueOnce(new Error('history offline')).mockResolvedValue({ runs: [] });
  render(<ScheduleRunsDrawer schedule={{ id: 'schedule-a' }} onClose={() => {}} onNotice={() => {}} />);
  await screen.findByText('history offline');
  expect(screen.queryByText('暂无触发记录')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '重试触发历史' }));
  await screen.findByText('暂无触发记录');
});
