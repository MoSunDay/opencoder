// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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

const action = (name) => screen.getByText(name, { selector: 'button > span' }).closest('button');

const overview = { goals: [], standalone_initiatives: [], backlog: [{ id: 'todo-1', title: '任务', draft: '说明', status: 'draft', capability_id: 'agent' }] };

beforeEach(() => {
  Object.values(api).forEach((method) => method.mockReset());
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { current_execution_id: 'agent-1', assignments: [{ execution_id: 'agent-1', kind: 'agent', name: '构建 Agent' }] }
    : path.endsWith('/result') ? { summary: '已完成' } : { id: 'agent-1', kind: 'agent', name: '构建 Agent', status: 'done' }));
  api.apiPost.mockResolvedValue({ execution_id: 'agent-2' });
  api.apiPatch.mockResolvedValue({ ok: true });
  api.apiDel.mockResolvedValue({ deleted: true });
});

it('displays execution type, name and ID resolved from the index', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  expect(screen.queryByText('execution:agent-1')).toBeNull();
  expect(await screen.findByText('构建 Agent')).toBeTruthy();
  expect(screen.getAllByText('已完成').length).toBeGreaterThan(0);
  expect(screen.getByText('执行结论')).toBeTruthy();
  expect(screen.getAllByText('agent-1').length).toBeGreaterThan(0);
  const link = screen.getByRole('link', { name: 'agent-1' });
  expect(link.getAttribute('href')).toBe('#execution=agent-1');
  expect(link.target).toBe('_blank');
  expect(link.rel).toContain('noopener');
  expect(screen.queryByRole('button', { name: '查看', exact: true })).toBeNull();
}, 20000);

it('removes manual execution linking while retaining capability selection and records', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  expect(screen.queryByLabelText('已有执行 ID')).toBeNull();
  expect(screen.queryByRole('button', { name: /^关\s*联$/ })).toBeNull();
  expect(screen.getByRole('combobox', { name: '执行能力' })).toBeTruthy();
  expect(await screen.findByText('构建 Agent')).toBeTruthy();
  expect(api.apiPost).not.toHaveBeenCalled();
});

it('opens an operator through its execution reference', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { current_execution_id: 'operator-1', assignments: [{ execution_id: 'operator-1', kind: 'operator' }] }
    : { id: 'operator-1', kind: 'operator', status: 'running' }));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  const link = await screen.findByRole('link', { name: 'operator-1' });
  expect(link.getAttribute('href')).toBe('#execution=operator-1');
  expect(link.target).toBe('_blank');
  expect(screen.queryByText('execution:operator-1')).toBeNull();
}, 20000);

it('submits Team guidance with a stable input ID from the execution detail', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path.endsWith('/executions')
    ? { current_execution_id: 'team-1', assignments: [{ execution_id: 'team-1', kind: 'team', name: '审核' }] }
    : path.includes('/comments') ? { comments: [{ seq: 1, body_md: '审核来源', author_name: 'alice', author_kind: 'user', execution_id: 'team-1', created_at: 1 }], has_more: false }
    : { id: 'team-1', kind: 'team', name: '审核', status: 'running' }));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(await screen.findByRole('button', { name: '查看执行来源' }));
  fireEvent.click(await screen.findByText('提交引导', { selector: 'button' }));
  await waitFor(() => expect(api.apiPost).toHaveBeenCalledWith('/api/executions/team-1/commands', {
    action: 'steer', input: { prompt: '补充说明', input_id: expect.stringMatching(/^input-/) },
  }));
}, 20000);

it('opens execution IDs without requiring a readable type or current index', async () => {
  api.apiGet.mockImplementation((path) => path.endsWith('/executions')
    ? Promise.resolve({ current_execution_id: 'lost-1', assignments: [{ execution_id: 'lost-1', kind: '', name: 'lost-1' }] })
    : Promise.reject(new Error('index unavailable')));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  await waitFor(() => expect(screen.getAllByText('lost-1').length).toBeGreaterThan(0));
  const link = screen.getByRole('link', { name: 'lost-1' });
  expect(link.getAttribute('href')).toBe('#execution=lost-1');
  expect(link.target).toBe('_blank');
  expect(screen.queryByRole('button', { name: '查看', exact: true })).toBeNull();
}, 20000);

it('saves TODO changes before opening the native capability UI', async () => {
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  fireEvent.click(screen.getByRole('tab', { name: '任务', exact: true }));
  fireEvent.click(within(screen.getByLabelText('任务说明显示方式')).getByText('编辑'));
  fireEvent.change(screen.getByLabelText('任务说明'), { target: { value: '新说明' } });
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  fireEvent.click(action('指派所选能力'));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/todo-1', expect.objectContaining({ draft: '新说明' })));
  expect((await screen.findByTestId('native-prompt')).textContent).toBe('任务\n\n新说明');
});

it('keeps the TODO open when saving before a native launch fails', async () => {
  api.apiPatch.mockRejectedValue(new Error('保存失败'));
  render(<TodoDrawer todoId="todo-1" overview={overview} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  fireEvent.click(action('指派所选能力'));
  expect(await screen.findByText('保存失败')).toBeTruthy();
  expect(screen.queryByTestId('native-prompt')).toBeNull();
});

it('saves a selected Operator before allowing assignment', async () => {
  api.apiGet.mockResolvedValue({ assignments: [], current_execution_id: null });
  const unbound = { ...overview, backlog: [{ ...overview.backlog[0], capability_id: null }] };
  render(<TodoDrawer todoId="todo-1" overview={unbound} refresh={vi.fn()} onClose={vi.fn()} onNotice={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  expect(action('指派所选能力').disabled).toBe(true);
  fireEvent.mouseDown(screen.getByRole('combobox', { name: '执行能力' }));
  fireEvent.click(await screen.findByText('Operator', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(action('保存 TODO'));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/todo-1', expect.objectContaining({ capability_id: 'operator' })));
  await waitFor(() => expect(action('指派所选能力').disabled).toBe(false));
}, 20000);
