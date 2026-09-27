// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
vi.mock('../fleet/detail.jsx', () => ({ ExecutionView: ({ executionRef }) => <div>execution:{executionRef.id}</div> }));
vi.mock('./execute/launcher.jsx', () => ({ CAPABILITIES: [{ value: 'agent', label: 'Agent' }], CapabilityLauncher: ({ onCreated }) => <button onClick={() => onCreated('agent-created')}>模拟发起</button> }));
import { TodoDrawer } from './todoDrawer.jsx';

const overview = { goals: [], standalone_milestones: [], backlog: [{ id: 'todo-1', title: '任务', draft: '说明', status: 'draft' }] };

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { execution_ids: ['agent-1'] }
    : { id: 'agent-1', kind: 'agent', name: '构建 Agent', status: 'done' }));
  api.apiPost.mockResolvedValue({ execution_id: 'agent-2' });
  api.apiPatch.mockResolvedValue({ ok: true });
  api.apiDel.mockResolvedValue({ deleted: true });
});

it('displays execution type, name and ID resolved from the index', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  expect(await screen.findByText('构建 Agent')).toBeTruthy();
  expect(screen.getAllByText('agent-1').length).toBeGreaterThan(0);
  fireEvent.click(screen.getByRole('button', { name: '查看' }));
  expect(await screen.findByText('execution:agent-1')).toBeTruthy();
}, 20000);

it('links an existing execution ID without storing a duplicate snapshot', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('已有执行 ID'), { target: { value: 'agent-2' } });
  fireEvent.click(screen.getByRole('button', { name: /关\s*联/ }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/todos/todo-1/executions', { execution_id: 'agent-2' }));
});

it('preserves a missing execution ID but does not open an unknown detail type', async () => {
  api.apiGet.mockImplementation((path) => path.endsWith('/executions')
    ? Promise.resolve({ execution_ids: ['lost-1'] })
    : Promise.reject(new Error('index unavailable')));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getAllByText('lost-1').length).toBeGreaterThan(0));
  expect(screen.getByRole('button', { name: '查看' }).disabled).toBe(true);
}, 20000);

it('retries auto-linking until a newly submitted execution reaches the index', async () => {
  api.apiPost.mockRejectedValueOnce(Object.assign(new Error('not indexed'), { status: 404 })).mockResolvedValue({ execution_id: 'agent-created' });
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: '从能力发起执行' }));
  fireEvent.click(screen.getByRole('button', { name: '模拟发起' }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledTimes(2), { timeout: 4000 });
  expect(api.apiPost).toHaveBeenCalledWith('/api/project/todos/todo-1/executions', { execution_id: 'agent-created' });
});
