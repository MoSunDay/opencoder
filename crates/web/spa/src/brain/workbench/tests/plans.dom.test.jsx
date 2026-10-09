// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiGet, apiPost } from '../../../api.js';
import { Plans } from '../plans.jsx';

vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
const capability = { id: 'coding', kind: 'agent', target: 'Coder', summary: '实现变更', version: '1', input_desc: '任务', output_desc: '结果', definition: {} };
const version = { id: 'delivery', version: 2, changelog: '调整计划', plan: {
  schema_version: 7, title: '交付计划', objective: '交付经过验证的变更', inputs: { revision: 'main' }, max_rounds: 5,
  layers: [{ layer_id: 'code', title: '开发', task: '实现需求', objective: '完成变更', success_criteria: '测试通过' }],
  nodes: [{ node_id: 'coding', layer_id: 'code', title: '编码', objective: '实现需求', capability_id: 'coding' }],
} };
const row = { id: version.id, title: version.plan.title, latest_version: version.version, schema_version: 7 };
const reload = vi.fn(); const onRun = vi.fn();
const renderPlans = (plans = [row]) => render(<Plans plans={plans} capabilities={[capability]} reload={reload} onRun={onRun} />);
beforeEach(() => {
  vi.resetAllMocks(); localStorage.clear();
  apiGet.mockImplementation(async (path) => path.endsWith('/versions') ? { versions: [version] } : version);
  apiPost.mockResolvedValue({ id: version.id, version: 3 });
});
afterEach(() => { cleanup(); localStorage.clear(); });

it('点击计划名称直接复用新建画布和表单，保存同一计划的新版本', async () => {
  renderPlans();
  fireEvent.click(screen.getByText('交付计划'));
  const drawer = await screen.findByRole('dialog', { name: '修改计划' });
  expect(drawer.closest('.ant-drawer-content-wrapper').style.width).toBe('100%');
  expect(drawer.querySelector('.brain-method-editor')).toBeTruthy();
  expect(drawer.querySelector('.brain-milestone-preview')).toBeNull();
  fireEvent.click(screen.getByText('下一步：计划信息'));
  expect(screen.getByLabelText('计划名称').value).toBe('交付计划');
  expect(screen.getByLabelText('整体目标与交付物').value).toBe(version.plan.objective);
  expect(screen.getByLabelText('工程参数值').value).toBe('"main"');
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: '更新交付计划' } });
  fireEvent.click(screen.getByText('保存计划版本'));
  await waitFor(() => expect(reload).toHaveBeenCalledOnce());
  expect(apiPost).toHaveBeenLastCalledWith('/api/brain/plan-defs', expect.objectContaining({
    id: 'delivery', version: 3, plan: expect.objectContaining({ title: '更新交付计划', inputs: { revision: 'main' } }),
  }));
  expect(version.plan.title).toBe('交付计划');
  expect(onRun).not.toHaveBeenCalled();
});

it('新建计划使用同一编辑器并保留关闭前的草稿', async () => {
  renderPlans([]);
  fireEvent.click(screen.getByText('新建计划'));
  const drawer = screen.getByRole('dialog', { name: '新建计划' });
  expect(drawer.querySelector('.brain-method-editor')).toBeTruthy();
  fireEvent.click(screen.getByText('添加第一个里程碑'));
  fireEvent.change(screen.getByLabelText('里程碑名称'), { target: { value: '草稿里程碑' } });
  fireEvent.click(screen.getByText('关闭画布'));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(screen.getByText('新建计划'));
  expect(screen.getByText('草稿里程碑')).toBeTruthy();
  expect(apiGet).not.toHaveBeenCalled();
  expect(apiPost).not.toHaveBeenCalled();
});

it('历史版本独立只读查看，执行引用该版本', async () => {
  renderPlans();
  fireEvent.click(screen.getByText('历史版本'));
  const drawer = await screen.findByRole('dialog', { name: '历史版本 · 交付计划' });
  expect(drawer.querySelector('.brain-milestone-preview')).toBeTruthy();
  expect(drawer.querySelector('.brain-method-editor')).toBeNull();
  fireEvent.click(screen.getByText('执行此版本'));
  expect(onRun).toHaveBeenCalledWith('delivery@2');
  expect(apiPost).not.toHaveBeenCalled();
});

it('旧格式计划只读打开，显式转换后才进入新版本编辑', async () => {
  const legacy = { ...version, plan: { ...version.plan, schema_version: 6, nodes: version.plan.nodes.map((node) => ({ ...node, layer: 1 })) } };
  apiGet.mockImplementation(async (path) => path.endsWith('/versions') ? { versions: [legacy] } : legacy);
  renderPlans([{ ...row, schema_version: 6 }]);
  fireEvent.click(screen.getByText('交付计划'));
  await screen.findByText('历史版本只读');
  expect(screen.queryByText('下一步：计划信息')).toBeNull();
  fireEvent.click(screen.getByRole('dialog').querySelector('.ant-drawer-close'));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(screen.getByText('转换为新版里程碑'));
  await waitFor(() => expect(apiGet).toHaveBeenCalledTimes(3));
  // antd uses the same generated title ID in tests, including hidden drawers.
  expect(await screen.findByText('修改计划', { selector: '.ant-drawer-title' })).toBeTruthy();
  expect(screen.getByText('下一步：计划信息')).toBeTruthy();
  expect(apiPost).not.toHaveBeenCalled();
});

it('读取失败不展示空白编辑表单，重新点击可重试', async () => {
  apiGet.mockRejectedValueOnce(new Error('计划读取失败'));
  renderPlans();
  fireEvent.click(screen.getByText('交付计划'));
  await screen.findByText('计划读取失败');
  expect(screen.queryByRole('dialog')).toBeNull();
  fireEvent.click(screen.getByText('交付计划'));
  expect(await screen.findByRole('dialog', { name: '修改计划' })).toBeTruthy();
});

it('现有计划的迟到读取不会覆盖已经打开的新建表单', async () => {
  let resolve;
  apiGet.mockImplementation(() => new Promise((done) => { resolve = done; }));
  renderPlans();
  fireEvent.click(screen.getByText('交付计划'));
  fireEvent.click(screen.getByText('新建计划'));
  resolve(version);
  await waitFor(() => expect(screen.getByRole('dialog', { name: '新建计划' })).toBeTruthy());
  expect(screen.queryByRole('dialog', { name: '修改计划' })).toBeNull();
});
