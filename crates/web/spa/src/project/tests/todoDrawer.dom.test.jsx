// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => <div>执行详情</div> }));
vi.mock('../execute/launcher.jsx', () => ({
  CAPABILITIES: [{ value: 'agent', label: 'Agent' }],
  CapabilityLauncher: ({ onCreated }) => <button onClick={() => onCreated('agent-late')}>发起验收</button>,
}));
vi.mock('../views/projectTable.jsx', () => ({
  ProjectTable: ({ rows }) => <div>{rows.map((row) => <div key={row.id}>{row.id}</div>)}</div>,
  TableText: ({ children }) => <span>{children}</span>,
}));
vi.mock('../execute/catalog.js', () => ({
  useCapabilities: () => ({ capabilities: [{ id: 'agent', kind: 'agent', target: 'Agent', definition: {} }, { id: 'operator', kind: 'operator', target: 'Operator', definition: {} }], loading: false, error: '' }),
  capabilityOptions: (caps) => caps.map((cap) => ({ value: cap.id, label: cap.target })),
}));
vi.mock('../execute/result.jsx', () => ({ ExecutionResult: () => null }));
import { TodoDrawer } from '../todoDrawer.jsx';

const overview = { goals: [], standalone_initiatives: [], tags: [],
  backlog: [{ id: 'todo', title: '任务', capability_id: 'agent', tag_ids: [] }] };
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
};
const drawer = () => render(<TodoDrawer todoId="todo" overview={overview}
  refresh={vi.fn().mockResolvedValue()} onClose={vi.fn()} onNotice={vi.fn()} />);

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiPatch.mockResolvedValue({});
  api.apiGet.mockResolvedValue({ assignments: [] });
});

it('keeps the TODO visible when a dispatched execution read completes after returning', async () => {
  const read = deferred();
  drawer();
  fireEvent.click(screen.getByRole('button', { name: '指派所选能力' }));
  await screen.findByRole('button', { name: '发起验收' });
  api.apiGet.mockImplementationOnce(() => read.promise);
  fireEvent.click(screen.getByRole('button', { name: '发起验收' }));
  fireEvent.click(screen.getByRole('button', { name: '返回 TODO' }));
  fireEvent.change(screen.getByLabelText('已有执行 ID'), { target: { value: 'agent-next' } });
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/index')
    ? { id: 'agent-late', kind: 'agent' } : { assignments: [{ execution_id: 'agent-late' }] }));
  await act(async () => read.resolve({ assignments: [{ execution_id: 'agent-late' }] }));
  expect(await screen.findByText('agent-late')).toBeTruthy();
  expect(api.apiPost).not.toHaveBeenCalled();
  expect(screen.queryByText('执行详情')).toBeNull();
  expect(screen.getByLabelText('已有执行 ID').value).toBe('agent-next');
});

it('ignores the old list response superseded by a successful new link', async () => {
  const old = deferred();
  let signal;
  api.apiGet.mockImplementationOnce((path, options) => { signal = options.signal; return old.promise; });
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/index')
    ? { id: 'agent-late', kind: 'agent' } : { assignments: [{ execution_id: 'agent-late' }] }));
  api.apiPost.mockResolvedValue({});
  drawer();
  fireEvent.change(screen.getByLabelText('已有执行 ID'), { target: { value: 'agent-late' } });
  fireEvent.click(screen.getByRole('button', { name: /^关\s*联$/ }));
  await screen.findByText('执行详情');
  expect(signal.aborted).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: '返回 TODO' }));
  await act(async () => old.resolve({ assignments: [] }));
  expect(screen.getByText('agent-late')).toBeTruthy();
});

it('aborts an unfinished list read when closing the TODO', async () => {
  const old = deferred();
  let signal;
  api.apiGet.mockImplementation((path, options) => { signal = options.signal; return old.promise; });
  const view = drawer();
  expect(signal.aborted).toBe(false);
  view.unmount();
  expect(signal.aborted).toBe(true);
  await act(async () => old.resolve({ assignments: [] }));
});

it('opens the current execution without dispatching or saving the TODO', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/index')
    ? { id: 'agent-current', kind: 'agent', status: 'running' }
    : { current_execution_id: 'agent-current', assignments: [{ execution_id: 'agent-current', kind: 'agent' }, { execution_id: 'old-dag', kind: 'dag' }] }));
  drawer();
  await screen.findByText('执行详情');
  expect(api.apiPost).not.toHaveBeenCalled();
  expect(api.apiPatch).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '返回 TODO' }));
  expect(screen.getByText('old-dag')).toBeTruthy();
});
