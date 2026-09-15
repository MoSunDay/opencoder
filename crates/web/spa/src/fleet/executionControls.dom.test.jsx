// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { ExecutionsPanel } from '../fleet/executions.jsx';
import { apiGet } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn() }));
vi.mock('../fleet/detail.jsx', () => ({ ExecutionDetail: () => null }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it('does not expose a per-run launch control on the execution index', async () => {
  apiGet.mockResolvedValue({ executions: [], nodes: [] });
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  expect(await screen.findByRole('button', { name: /刷\s*新/ })).toBeTruthy();
  expect(screen.queryByRole('button', { name: '启动执行' })).toBeNull();
  expect(screen.queryByRole('button', { name: '加载更早的执行' })).toBeNull();
});
