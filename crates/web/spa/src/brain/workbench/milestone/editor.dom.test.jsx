// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiPost } from '../../../api.js';
import { PlanEditor } from '../scheduler/editor.jsx';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn() }));
afterEach(() => { cleanup(); localStorage.clear(); vi.clearAllMocks(); });
const cap = { id: 'coding', kind: 'agent', target: 'Coder', summary: '编码能力', version: '1', input_desc: '任务', output_desc: '变更', definition: {} };
const version = { id: 'plan-canvas', version: 1, plan: { schema_version: 5, title: '交付计划', objective: '交付经过验证的变更', inputs: {}, max_rounds: 5,
  nodes: [{ node_id: 'code', title: 'Coding', layer: 1, objective: '实现变更', success_criteria: '实现完成', capability_ids: ['coding'] }], edges: [] } };
it('在真实画布编辑节点后，表单提交保存新版本且不会启动运行', async () => {
  const saved = vi.fn(); apiPost.mockResolvedValue({ id: 'plan-canvas', version: 2 });
  const { container } = render(<PlanEditor version={version} cacheKey="canvas-test" capabilities={[cap]} onSaved={saved} onClose={() => {}} />);
  fireEvent.click(container.querySelector('.react-flow__node'));
  fireEvent.change(screen.getByLabelText('达成标准'), { target: { value: '通过测试和审查' } });
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
  expect(body.plan.nodes[0].success_criteria).toBe('通过测试和审查');
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
  expect(screen.getByText('Coding')).toBeTruthy();
  expect(apiPost).toHaveBeenCalledTimes(1);
});
