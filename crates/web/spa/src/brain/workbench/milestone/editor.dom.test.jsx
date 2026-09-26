// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiPost } from '../../../api.js';
import { PlanEditor } from '../scheduler/editor.jsx';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn() }));
afterEach(() => { cleanup(); localStorage.clear(); vi.clearAllMocks(); });
const cap = { id: 'coding', kind: 'agent', target: 'Coder', summary: '编码能力', version: '1', input_desc: '任务', output_desc: '变更', definition: {} };
const version = { id: 'plan-canvas', version: 1, plan: { schema_version: 7, title: '交付计划', objective: '交付经过验证的变更', inputs: {}, max_rounds: 5,
  layers: [{ layer_id: 'coding', title: 'Coding', objective: '实现变更', success_criteria: '实现完成' }], nodes: [{ node_id: 'code', title: 'Coding 任务', layer_id: 'coding', objective: '实现变更', capability_id: 'coding' }], transitions: [] } };
it('在真实画布编辑节点后，表单提交保存新版本且不会启动运行', async () => {
  const saved = vi.fn(); apiPost.mockResolvedValue({ id: 'plan-canvas', version: 2 });
  const { container } = render(<PlanEditor version={version} cacheKey="canvas-test" capabilities={[cap]} onSaved={saved} onClose={() => {}} />);
  fireEvent.click(container.querySelector('.react-flow__node-layer'));
  fireEvent.change(screen.getByLabelText('里程碑达成标准'), { target: { value: '通过测试和审查' } });
  expect(apiPost).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText('下一步：计划信息'));
  await screen.findByText('计划信息与提交');
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: '新的交付方法论' } });
  fireEvent.click(screen.getByText('保存计划版本'));
  await waitFor(() => expect(saved).toHaveBeenCalledTimes(1));
  const [path, body] = apiPost.mock.calls[1];
  expect(path).toBe('/api/brain/plan-defs');
  expect(body.version).toBe(2);
  expect(body.plan.title).toBe('新的交付方法论');
  expect(body.plan.layers[0].success_criteria).toBe('通过测试和审查');
  expect(body.plan.max_rounds).toBe(5);
  expect(apiPost.mock.calls).toHaveLength(2);
  expect(localStorage.getItem('canvas-test')).toBeNull();
});
it('提交失败后保留节点与表单草稿，返回画布可继续修改', async () => {
  apiPost.mockRejectedValue(new Error('服务端校验失败'));
  render(<PlanEditor version={version} cacheKey="canvas-failed" capabilities={[cap]} onSaved={vi.fn()} onClose={() => {}} />);
  fireEvent.click(screen.getByText('下一步：计划信息'));
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: '保留我的编辑' } });
  fireEvent.click(screen.getByText('保存计划版本'));
  await waitFor(() => expect(screen.getAllByText('服务端校验失败').length).toBeGreaterThan(0));
  expect(JSON.parse(localStorage.getItem('canvas-failed')).version.plan.title).toBe('保留我的编辑');
  fireEvent.click(screen.getByText('返回画布'));
  expect(screen.getAllByText('Coding').length).toBeGreaterThan(0);
  expect(apiPost).toHaveBeenCalledTimes(1);
});
it('在里程碑容器内添加并配置并行执行节点', async () => {
  const { container } = render(<PlanEditor version={version} cacheKey="canvas-parallel" capabilities={[cap]} onSaved={vi.fn()} onClose={() => {}} />);
  fireEvent.click(screen.getByText('＋ 并行执行节点'));
  fireEvent.change(screen.getByLabelText('执行节点名称'), { target: { value: '补充审查' } });
  fireEvent.change(screen.getByLabelText('执行节点任务'), { target: { value: '并行审查变更' } });
  fireEvent.mouseDown(screen.getByLabelText('绑定能力'));
  fireEvent.click((await screen.findAllByText('Agent · Coder')).find((item) => item.classList.contains('ant-select-item-option-content')));
  const draft = JSON.parse(localStorage.getItem('canvas-parallel'));
  expect(draft.version.plan.nodes).toHaveLength(2);
  expect(draft.version.plan.nodes[1]).toMatchObject({ layer_id: 'coding', title: '补充审查', objective: '并行审查变更', capability_id: 'coding' });
  expect(container.querySelectorAll('.react-flow__node-execution')).toHaveLength(2);
  expect(container.querySelector('.brain-execution-node span')?.textContent).toBe('Agent · Coder');
});
