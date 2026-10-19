// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { App, ConfigProvider } from 'antd';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { DraftGuardProvider } from '../../../ontology/navigation/DraftGuard';
import { pushLocation } from '../../../fleet/navigation/history.js';
import { useLinkedLocation } from '../../../fleet/navigation/location.js';
import { BrainWorkbench } from '../index.jsx';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(async () => ({ plans: [], runs: [{ id: 'run1' }], capabilities: [] })), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('../useRun.js', () => ({ useBrainRun: () => ({ run: null, error: '', refresh: vi.fn() }) }));
afterEach(() => window.history.replaceState(null, '', '/'));
function Location() { const { page } = useLinkedLocation(); return <output>{page || '无定位'}</output>; }

it('opening and closing a Brain run updates the shared location without adding a history entry', async () => {
  window.history.replaceState(null, '', '/');
  render(<ConfigProvider theme={{ token: { motion: false } }}><App><DraftGuardProvider>
    <BrainWorkbench onNotice={vi.fn()} /><Location />
  </DraftGuardProvider></App></ConfigProvider>);
  act(() => pushLocation('/?keep=1'));
  fireEvent.click(await screen.findByText('查看运行'));
  await waitFor(() => expect(window.location.search).toBe('?keep=1&brain_run=run1'));
  expect(screen.getByRole('status').textContent).toBe('brain');
  fireEvent.click(screen.getByLabelText('返回运行历史'));
  await waitFor(() => expect(window.location.search).toBe('?keep=1'));
  expect(screen.getByRole('status').textContent).toBe('无定位');
  act(() => window.history.back());
  await waitFor(() => expect(window.location.search).toBe(''));
}, 10000);
