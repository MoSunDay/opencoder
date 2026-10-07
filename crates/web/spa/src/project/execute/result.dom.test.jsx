// @vitest-environment jsdom
import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn() }));
vi.mock('../../api.js', () => api);
import { ExecutionResult } from './result.jsx';
import { capabilityOptions, useCapabilities } from './catalog.js';
beforeEach(() => api.apiGet.mockReset());
it('displays node failure and retries without retaining a previous success', async () => {
  api.apiGet.mockRejectedValueOnce(Object.assign(new Error('offline'), { status: 503 })).mockResolvedValue({ summary: '最新结论' });
  render(<ExecutionResult id="agent-1" />);
  await screen.findByText('执行节点离线，恢复后可读取最新结论');
  fireEvent.click(screen.getByRole('button', { name: '重试读取' }));
  await screen.findByText('最新结论');
  expect(api.apiGet.mock.calls.every(([path]) => path === '/api/executions/agent-1/result')).toBe(true);
});
it('does not replace a different execution with a late result and aborts old reads', async () => {
  let resolve, signal;
  api.apiGet.mockImplementationOnce((_, options) => { signal = options.signal; return new Promise((done) => { resolve = done; }); });
  api.apiGet.mockResolvedValue({ summary: 'current' });
  const view = render(<ExecutionResult id="agent-old" />);
  view.rerender(<ExecutionResult id="agent-current" />);
  await screen.findByText('current');
  expect(signal.aborted).toBe(true);
  await act(async () => resolve({ summary: 'stale' }));
  expect(screen.queryByText('stale')).toBeNull();
});
it('keeps capability read failures visible and reloads concrete IDs', async () => {
  function Catalog() { const state = useCapabilities(); return <button onClick={state.reload}>{state.error || state.capabilities.map((cap) => cap.id).join(',')}</button>; }
  api.apiGet.mockRejectedValueOnce(new Error('catalog unavailable')).mockResolvedValue({ capabilities: [{ id: 'registered-employee' }] });
  render(<Catalog />);
  fireEvent.click(await screen.findByText('catalog unavailable'));
  await screen.findByText('registered-employee');
  expect(capabilityOptions([{ id: 'missing', target: 'name', definition: null }])[0]).toMatchObject({ value: 'missing', disabled: true });
});
