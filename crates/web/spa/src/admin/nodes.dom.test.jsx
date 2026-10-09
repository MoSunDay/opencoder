// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import '../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
vi.mock('../fleet/detail.jsx', () => ({ ExecutionDetail: () => null }));
import { FleetNodesPanel } from '../fleet/nodes.jsx';
afterEach(() => { cleanup(); vi.resetAllMocks(); });
it('combines node search and online status while preserving action availability', async () => {
  api.apiGet.mockResolvedValue({ nodes: [
    { id: 'node-a', name: 'worker-a', online: true, snapshot: { ready: true, cpu_capacity: 2, active_agent_loops: 3 } },
    { id: 'node-b', name: 'worker-b', online: false },
  ] });
  render(<FleetNodesPanel onNotice={vi.fn()} />);
  await screen.findByText('worker-a');
  const offline = within(screen.getByText('worker-b').closest('tr'));
  expect(offline.getByRole('button', { name: '维护节点' }).disabled).toBe(true);
  expect(offline.getByRole('button', { name: '删除节点' }).disabled).toBe(false);
  fireEvent.change(screen.getByLabelText('搜索节点名称或 ID'), { target: { value: 'NODE-B' } });
  expect(screen.queryByText('worker-a')).toBeNull();
  expect(screen.getByText('worker-b')).toBeTruthy();
  fireEvent.mouseDown(screen.getByRole('combobox', { name: '筛选节点状态' }));
  fireEvent.click(await screen.findByText('在线', { selector: '.ant-select-item-option-content' }));
  expect(screen.getByText('没有匹配的节点')).toBeTruthy();
  expect(api.apiPost).not.toHaveBeenCalled();
});
it('shows unknown CPU-normalized load instead of Infinity or NaN', async () => {
  api.apiGet.mockResolvedValue({ nodes: [{ id: 'zero', name: 'no-cpu', online: false, snapshot: { cpu_capacity: 0, active_agent_loops: 1 } }] });
  render(<FleetNodesPanel onNotice={vi.fn()} />);
  await screen.findByText('no-cpu');
  expect(screen.getByText('loops / CPU：').textContent).toBe('loops / CPU：—');
});
