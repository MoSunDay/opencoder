// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiPost } from '../../../api.js';
import { MilestoneRunBody } from './run.jsx';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn() }));
vi.mock('../../../fleet/detail.jsx', () => ({ ExecutionView: ({ executionRef }) => <div data-testid="execution-panel">{executionRef.kind}:{executionRef.id}</div> }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
function view(kind) {
  const op = (activation) => ({ activation, round: activation, layer: 1, operation_id: `operation-${activation}`, node_id: 'code', capability_id: 'cap', execution_kind: kind, execution_id: `${kind}-visit-${activation}`, status: activation === 1 ? 'error' : 'running' });
  return { schema_version: 7, plan: { schema_version: 7, title: '闭环计划', objective: '交付', layers: [{ layer_id: 'coding', title: 'Coding', objective: '整改', success_criteria: '验证通过' }], nodes: [{ node_id: 'code', layer_id: 'coding', title: 'Coding 任务', objective: '整改', capability_id: 'cap' }], transitions: [{ from: 'coding', to: 'coding', condition: '需整改' }] },
    run: { phase: 'waiting', round: 2, max_rounds: 5, activation: 2, layer: 1, valid_layers: 0 }, layers: [['code']], operations: [op(1), op(2)],
    events: [1, 2].map((activation) => ({ activation, round: activation, layer: 1, event_type: 'layer_started', decision_summary: activation === 1 ? 'dispatch_layer' : 'reflect_and_return', reason_summary: '派发依据', reflection: activation === 2 ? '修复首轮问题' : null, evidence_execution_ids: activation === 1 ? [] : [`${kind}-visit-1`], assignments: [{ node_id: 'code', capability_id: 'cap', inputs: { task: { kind: 'value', value: `任务-${activation}` } } }] })) };
}
it.each(['agent', 'team', 'dag', 'todos', 'operator', 'brain'])('按最新轮的类型与 ID 打开已有 %s 执行面板', async (kind) => {
  const { container } = render(<MilestoneRunBody view={view(kind)} id="brain-run" refresh={vi.fn()} />);
  fireEvent.click(container.querySelector('.react-flow__node-execution'));
  expect((await screen.findByTestId('execution-panel')).textContent).toBe(`${kind}:${kind}-visit-2`);
});
it('历史轮保留各自输入、反思和执行 ID', () => {
  render(<MilestoneRunBody view={view('agent')} id="brain-run" refresh={vi.fn()} />);
  fireEvent.click(screen.getByText('第 1 轮 · 第 1 层 · 1 项执行'));
  fireEvent.click(screen.getByText('第 2 轮 · 第 1 层 · 1 项执行'));
  expect(screen.getByText('agent-visit-1')).toBeTruthy();
  expect(screen.getByText('agent-visit-2')).toBeTruthy();
  expect(screen.getByText('修复首轮问题')).toBeTruthy();
  expect(screen.getByText(/任务-1/)).toBeTruthy();
  expect(screen.getByText(/任务-2/)).toBeTruthy();
});
it('耗尽预算后可以显式增加预算并继续调度', async () => {
  const data = view('agent'); data.run = { ...data.run, round: 5, phase: 'blocked', error: 'round budget exhausted' };
  const refresh = vi.fn(); apiPost.mockResolvedValue({});
  render(<MilestoneRunBody view={data} id="brain-run" refresh={refresh} />);
  fireEvent.change(screen.getByLabelText('新的轮次预算'), { target: { value: '8' } });
  fireEvent.click(screen.getByText('调整预算'));
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
  expect(apiPost).toHaveBeenLastCalledWith('/api/brain/runs/brain-run/commands', { action: 'set_round_budget', input: { max_rounds: 8 } });
  fireEvent.click(screen.getByText('继续调度'));
  await waitFor(() => expect(apiPost).toHaveBeenLastCalledWith('/api/brain/runs/brain-run/commands', { action: 'resume', input: {} }));
});
