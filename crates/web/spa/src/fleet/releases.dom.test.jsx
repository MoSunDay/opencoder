// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { ReleasesModal } from './settings/releases.jsx';
import { apiGet } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it('shows retained task ownership, sleep state and independent retirement failures', async () => {
  apiGet.mockResolvedValue({ release: { phase: 'complete', current: 'r3', candidate: null,
    retirement: { r1: { failure: 'connection lost' } } }, host: { runtimes: [
      { runtime: { id: 'r1', release_id: 'r1', mode: 'retired' }, remaining: [['ticket', 'agent-old', 'running']], hibernated: false },
      { runtime: { id: 'r2', release_id: 'r2', mode: 'retired' }, remaining: [], hibernated: true },
      { runtime: { id: 'r3', release_id: 'r3', mode: 'active' }, remaining: [] },
    ] } });
  const inspect = vi.fn();
  render(<ReleasesModal onClose={vi.fn()} onInspect={inspect} />);
  expect(await screen.findByText('发布完成')).toBeTruthy();
  expect(screen.getByText('休眠，访问时恢复')).toBeTruthy();
  expect(screen.getByText('r1 服务退役失败：connection lost')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agent-old · 执行中' }));
  expect(inspect).toHaveBeenCalledWith('agent-old');
});

it('reports a failed release status read', async () => {
  apiGet.mockRejectedValue(new Error('host unavailable'));
  render(<ReleasesModal onClose={vi.fn()} onInspect={vi.fn()} />);
  expect(await screen.findByText('host unavailable')).toBeTruthy();
  expect(screen.queryByText('发布完成')).toBeNull();
});
