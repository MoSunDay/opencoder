// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPatch: vi.fn(), apiDel: vi.fn() }));
vi.mock('../api.js', () => api);
vi.mock('../fleet/detail.jsx', () => ({ ExecutionView: ({ executionRef, onGuidance }) => <div>execution:{executionRef.id}{executionRef.kind === 'team' && <button onClick={() => onGuidance('补充说明')}>提交引导</button>}</div> }));
vi.mock('./execute/launcher.jsx', () => ({
  CAPABILITIES: [{ value: 'agent', label: 'Agent' }, { value: 'operator', label: 'Operator' }],
  CapabilityLauncher: ({ prompt }) => <div data-testid="native-prompt">{prompt}</div>,
}));
vi.mock('./execute/catalog.js', () => ({
  useCapabilities: () => ({ capabilities: [{ id: 'agent', kind: 'agent', target: 'Agent', definition: {} }, { id: 'operator', kind: 'operator', target: 'Operator', definition: {} }], loading: false, error: '' }),
  capabilityOptions: (caps) => caps.map((cap) => ({ value: cap.id, label: cap.target })),
}));
import { TodoDrawer } from './todoDrawer.jsx';

const overview = { goals: [], standalone_initiatives: [], backlog: [{ id: 'todo-1', title: '任务', draft: '说明', status: 'draft', capability_id: 'agent' }] };

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { assignments: [{ execution_id: 'agent-1', kind: 'agent', name: '构建 Agent' }] }
    : path.endsWith('/result') ? { summary: '已完成' } : { id: 'agent-1', kind: 'agent', name: '构建 Agent', status: 'done' }));
  api.apiPost.mockResolvedValue({ execution_id: 'agent-2' });
  api.apiPatch.mockResolvedValue({ ok: true });
  api.apiDel.mockResolvedValue({ deleted: true });
});

it('displays execution type, name and ID resolved from the index', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  expect(await screen.findByText('构建 Agent')).toBeTruthy();
  expect(screen.getAllByText('已完成').length).toBeGreaterThan(0);
  expect(screen.getByText('执行结论')).toBeTruthy();
  expect(screen.getAllByText('agent-1').length).toBeGreaterThan(0);
  fireEvent.click(screen.getByText('查看', { selector: 'button span' }).closest('button'));
  expect(await screen.findByText('execution:agent-1')).toBeTruthy();
}, 20000);

it('links an existing execution ID', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('已有执行 ID'), { target: { value: 'agent-2' } });
  fireEvent.click(screen.getByRole('button', { name: /关\s*联/ }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/project/todos/todo-1/executions', { execution_id: 'agent-2' }));
});

it('opens an operator through its execution reference', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { assignments: [{ execution_id: 'operator-1', kind: 'operator' }] }
    : { id: 'operator-1', kind: 'operator', status: 'running' }));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  expect((await screen.findAllByText('operator-1')).length).toBeGreaterThan(0);
  fireEvent.click(screen.getByText('查看', { selector: 'button span' }).closest('button'));
  expect(await screen.findByText('execution:operator-1')).toBeTruthy();
}, 20000);

it('submits Team guidance with a stable input ID from the execution detail', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { assignments: [{ execution_id: 'team-1', kind: 'team', name: '审核' }] }
    : { id: 'team-1', kind: 'team', name: '审核', status: 'running' }));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  const view = (await screen.findByText('查看', { selector: 'button span' })).closest('button');
  await waitFor(() => expect(view.disabled).toBe(false));
  fireEvent.click(view);
  fireEvent.click(await screen.findByText('提交引导', { selector: 'button' }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/executions/team-1/commands', {
    action: 'steer', input: { prompt: '补充说明', input_id: expect.stringMatching(/^input-/) },
  }));
}, 20000);

it('preserves a missing execution ID but does not open an unknown detail type', async () => {
  api.apiGet.mockImplementation((path) => path.endsWith('/executions')
    ? Promise.resolve({ assignments: [{ execution_id: 'lost-1', kind: '', name: 'lost-1' }] })
    : Promise.reject(new Error('index unavailable')));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  await waitFor(() => expect(screen.getAllByText('lost-1').length).toBeGreaterThan(0));
  expect(screen.getByRole('button', { name: '查看' }).disabled).toBe(true);
}, 20000);

it('saves TODO changes before opening the native capability UI', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '新说明' } });
  fireEvent.click(screen.getByRole('button', { name: '指派所选能力' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/todo-1', expect.objectContaining({ draft: '新说明' })));
  expect((await screen.findByTestId('native-prompt')).textContent).toBe('任务\n\n新说明');
});

it('keeps the TODO open when saving before a native launch fails', async () => {
  api.apiPatch.mockRejectedValue(new Error('保存失败'));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: '指派所选能力' }));
  expect(await screen.findByText('保存失败')).toBeTruthy();
  expect(screen.queryByTestId('native-prompt')).toBeNull();
});

it('saves a selected Operator before allowing assignment', async () => {
  const unbound = { ...overview, backlog: [{ ...overview.backlog[0], capability_id: null }] };
  render(<TodoDrawer todoId="todo-1" overview={unbound} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  expect(screen.getByRole('button', { name: '指派所选能力' }).disabled).toBe(true);
  fireEvent.mouseDown(screen.getByRole('combobox', { name: '执行能力' }));
  fireEvent.click(await screen.findByText('Operator', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/todo-1', expect.objectContaining({ capability_id: 'operator' })));
  await waitFor(() => expect(screen.getByRole('button', { name: /指派所选能力/ }).disabled).toBe(false));
}, 20000);
