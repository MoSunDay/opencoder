// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiPost } from '../../../api.js';
import { MilestoneRunBody } from './run.jsx';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn() }));
vi.mock('../../../fleet/detail.jsx', () => ({ ExecutionView: ({ executionRef, allowGuidance, onGuidance }) => <div data-testid="execution-panel">
  <span data-testid="execution-identity">{executionRef.kind}:{executionRef.id}:{String(allowGuidance)}</span><button onClick={() => onGuidance('执行详情引导')}>提交执行引导</button>
</div> }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
function view(kind) {
  const op = (activation) => ({ activation, round: activation, layer: 1, operation_id: `operation-${activation}`, node_id: 'code', capability_id: 'cap', execution_kind: kind, execution_id: `${kind}-visit-${activation}`, status: activation === 1 ? 'error' : 'running' });
  return { schema_version: 7, plan: { schema_version: 7, title: '闭环计划', objective: '交付', layers: [{ layer_id: 'coding', title: 'Coding', task: '整改', objective: '整改', success_criteria: '验证通过' }], nodes: [{ node_id: 'code', layer_id: 'coding', title: 'Coding 任务', objective: '整改', capability_id: 'cap' }], transitions: [{ from: 'coding', to: 'coding', condition: '需整改' }] },
    run: { phase: 'waiting', round: 2, max_rounds: 5, activation: 2, layer: 1, valid_layers: 0 }, layers: [['code']], operations: [op(1), op(2)],
    events: [1, 2].map((activation) => ({ activation, round: activation, layer: 1, event_type: 'layer_started', decision_summary: activation === 1 ? 'dispatch_layer' : 'reflect_and_return', reason_summary: '派发依据', reflection: activation === 2 ? '修复首轮问题' : null, evidence_execution_ids: activation === 1 ? [] : [`${kind}-visit-1`], assignments: [{ node_id: 'code', capability_id: 'cap', inputs: { task: { kind: 'value', value: `任务-${activation}` } } }] })) };
}
it.each(['agent', 'team', 'dag', 'todos', 'operator', 'brain'])('按最新轮的类型与 ID 打开已有 %s 执行面板', async (kind) => {
  const { container } = render(<MilestoneRunBody view={view(kind)} id="brain-run" refresh={vi.fn()} />);
  fireEvent.click(container.querySelector('.react-flow__node-execution'));
  expect((await screen.findByTestId('execution-identity')).textContent).toBe(`${kind}:${kind}-visit-2:true`);
});
it('人工输入从运行页送入大脑事件', async () => {
  const refresh = vi.fn(); apiPost.mockResolvedValue({});
  render(<MilestoneRunBody view={view('agent')} id="brain-run" refresh={refresh} />);
  expect(screen.queryByLabelText('大脑人工输入')).toBeNull();
  fireEvent.click(screen.getByLabelText('打开大脑对话'));
  fireEvent.change(screen.getByLabelText('大脑人工输入'), { target: { value: '优先核对验证结果' } });
  fireEvent.click(screen.getByRole('button', { name: '发送大脑输入' }));
  await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/brain/runs/brain-run/inputs',
    { text: '优先核对验证结果' }));
  expect(refresh).toHaveBeenCalled();
});
it('大脑对话按 UTF-8 字节限制输入并保留超限草稿', async () => {
  render(<MilestoneRunBody view={view('agent')} id="brain-run" refresh={vi.fn()} />);
  fireEvent.click(screen.getByLabelText('打开大脑对话'));
  const input = screen.getByLabelText('大脑人工输入');
  fireEvent.change(input, { target: { value: '中'.repeat(1366) } });
  expect(screen.getByText('4098 / 4096 字节')).toBeTruthy();
  expect(screen.getByRole('button', { name: '发送大脑输入' }).disabled).toBe(true);
  expect(apiPost).not.toHaveBeenCalled();
  expect(input.value).toHaveLength(1366);
});
it('输入已记录但刷新失败时清空草稿并提示刷新错误', async () => {
  apiPost.mockResolvedValue({});
  render(<MilestoneRunBody view={view('agent')} id="brain-run" refresh={vi.fn().mockRejectedValue(new Error('连接中断'))} />);
  fireEvent.click(screen.getByText('查看详情'));
  fireEvent.change(screen.getByLabelText('大脑人工输入'), { target: { value: '更新判断依据' } });
  fireEvent.click(screen.getByRole('button', { name: '发送大脑输入' }));
  await waitFor(() => expect(screen.getByLabelText('大脑人工输入').value).toBe(''));
  expect(screen.getByText('输入已记录，刷新失败：连接中断')).toBeTruthy();
});
it('执行详情的引导也只发送大脑输入事件', async () => {
  apiPost.mockResolvedValue({});
  render(<MilestoneRunBody view={view('agent')} id="brain-run" refresh={vi.fn()} />);
  fireEvent.click(screen.getByText('查看详情'));
  fireEvent.click(screen.getByText('第 2 轮 · 1 层 · 1 项执行'));
  fireEvent.click(screen.getByText('agent-visit-2'));
  fireEvent.click(screen.getByText('提交执行引导'));
  await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/brain/runs/brain-run/inputs', { text: '执行详情引导' }));
  expect(apiPost).toHaveBeenCalledTimes(1);
});
it('顶部仅显示状态操作和画布，右侧详情按轮次汇总层级执行记录', async () => {
  render(<MilestoneRunBody view={view('agent')} id="brain-run" refresh={vi.fn()} />);
  expect(screen.queryByRole('heading', { name: '闭环计划' })).toBeNull();
  expect(screen.queryByText('交付')).toBeNull();
  fireEvent.click(screen.getByText('查看详情'));
  expect(await screen.findByRole('heading', { name: '闭环计划' })).toBeTruthy();
  fireEvent.click(screen.getByText('第 1 轮 · 1 层 · 1 项执行'));
  expect(screen.getByText('agent-visit-1')).toBeTruthy();
  fireEvent.click(screen.getByText('第 1 层 · 调度依据'));
  expect(screen.getByText('派发依据')).toBeTruthy();
  expect(screen.getByText(/任务-1/)).toBeTruthy();
  fireEvent.click(screen.getByText('agent-visit-1'));
  expect((await screen.findByTestId('execution-identity')).textContent).toBe('agent:agent-visit-1:true');
  fireEvent.click(screen.getByText('返回轮次列表'));
  fireEvent.click(screen.getByText('第 2 轮 · 1 层 · 1 项执行'));
  expect(screen.getByText('agent-visit-2')).toBeTruthy();
  fireEvent.click(screen.getByText('第 1 层 · 调度依据'));
  expect(screen.getByText('修复首轮问题')).toBeTruthy();
});
it('同一轮的多层执行在同一张表中展示', async () => {
  const data = view('agent');
  data.plan.layers.push({ layer_id: 'verify', title: 'Verify', task: '验证', objective: '验证', success_criteria: '通过' });
  data.plan.nodes.push({ node_id: 'check', layer_id: 'verify', title: '验证任务', objective: '验证', capability_id: 'cap' });
  data.plan.transitions = [{ from: 'coding', to: 'verify', condition: '编码完成' }];
  data.operations.push({ ...data.operations[1], activation: 3, round: 2, layer: 2, operation_id: 'operation-3', node_id: 'check', execution_id: 'agent-visit-3' });
  data.events.push({ activation: 3, round: 2, layer: 2, event_type: 'layer_started', evidence_execution_ids: [] });
  data.run = { ...data.run, activation: 3, layer: 2 };
  render(<MilestoneRunBody view={data} id="brain-run" refresh={vi.fn()} />);
  fireEvent.click(screen.getByText('查看详情'));
  fireEvent.click(await screen.findByText('第 2 轮 · 2 层 · 2 项执行'));
  expect(screen.getByText('agent-visit-2')).toBeTruthy();
  expect(screen.getByText('agent-visit-3')).toBeTruthy();
  expect(screen.getByText('第 2 层 · Verify')).toBeTruthy();
});
it('耗尽预算后可以显式增加预算并继续调度', async () => {
  const data = view('agent'); data.run = { ...data.run, round: 5, phase: 'blocked', error: 'round budget exhausted' };
  const refresh = vi.fn(); apiPost.mockResolvedValue({});
  render(<MilestoneRunBody view={data} id="brain-run" refresh={refresh} />);
  fireEvent.click(screen.getByText('查看详情'));
  fireEvent.change(screen.getByLabelText('新的轮次预算'), { target: { value: '8' } });
  fireEvent.click(screen.getByText('调整预算'));
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
  expect(apiPost).toHaveBeenLastCalledWith('/api/brain/runs/brain-run/commands', { action: 'set_round_budget', input: { max_rounds: 8 } });
  fireEvent.click(screen.getByText('继续调度'));
  await waitFor(() => expect(apiPost).toHaveBeenLastCalledWith('/api/brain/runs/brain-run/commands', { action: 'resume', input: {} }));
});
