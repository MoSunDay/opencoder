// @vitest-environment jsdom
import '../../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { PlanEditor, PlanPreview } from '../../scheduler/editor.jsx';
import { apiPost } from '../../../../api.js';
import { capability, version } from './fixtures.js';
vi.mock('../../../../api.js', () => ({ apiPost: vi.fn() }));
afterEach(() => { cleanup(); localStorage.clear(); vi.clearAllMocks(); });
const key = 'parallel-draft';
const draft = () => JSON.parse(localStorage.getItem(key));
const mount = () => render(<PlanEditor cacheKey={key} version={version} capabilities={[capability]} onSaved={vi.fn()} onClose={vi.fn()} />);
const item = (container, id) => container.querySelector(`[data-id="${id}"]`);
async function confirm() {
  const dialog = (await screen.findByText('删除选中的里程碑或执行节点？')).closest('[role="dialog"]');
  fireEvent.click(within(dialog).getByRole('button', { name: /删\s*除/ }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
}

it.each(['Backspace', 'Delete'])('%s 删除同层节点后继续编辑、重开和保存均不恢复节点', async (keyName) => {
  apiPost.mockResolvedValue({ id: version.id, version: 2 });
  let view = mount();
  fireEvent.click(item(view.container, 'execution:code-a'));
  fireEvent.keyDown(item(view.container, 'execution:code-a'), { key: keyName });
  await confirm();
  fireEvent.click(item(view.container, 'execution:code-b'));
  expect(item(view.container, 'execution:code-a')).toBeNull();
  expect(draft().version.plan.nodes.map((n) => n.node_id)).toEqual(['code-b', 'test-a']);
  view.unmount(); view = mount();
  expect(item(view.container, 'execution:code-a')).toBeNull();
  fireEvent.click(item(view.container, 'layer:coding'));
  fireEvent.change(screen.getByLabelText('里程碑要做什么'), { target: { value: '删除后继续修改' } });
  fireEvent.click(screen.getByText('下一步：计划信息'));
  fireEvent.click(screen.getByText('保存计划版本'));
  await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(2));
  expect(apiPost.mock.calls[1][1].plan.nodes.map((n) => n.node_id)).toEqual(['code-b', 'test-a']);
  expect(apiPost.mock.calls[1][1].plan.layers[0].task).toBe('删除后继续修改');
});

it('取消键盘删除和输入框退格不修改计划', async () => {
  const { container } = mount();
  fireEvent.click(item(container, 'layer:coding'));
  const before = draft();
  fireEvent.keyDown(screen.getByLabelText('里程碑名称'), { key: 'Backspace' });
  expect(screen.queryByRole('dialog')).toBeNull();
  fireEvent.keyDown(item(container, 'layer:coding'), { key: 'Delete' });
  fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: /取\s*消/ }));
  expect(draft()).toEqual(before);
  expect(item(container, 'execution:code-a')).toBeTruthy();
});

it('按钮删除子节点保留各层坐标，删除整层仅清理该层，整理布局才清空坐标', async () => {
  const layout = { coding: { x: 600, y: 200 }, testing: { x: 50, y: 650 } };
  localStorage.setItem(key, JSON.stringify({ version: { ...version, version: 2 }, engineering: [], layout }));
  const { container } = mount();
  fireEvent.click(item(container, 'execution:code-a'));
  fireEvent.click(screen.getByText('删除执行节点')); await confirm();
  expect(draft().layout).toEqual(layout);
  fireEvent.click(item(container, 'layer:coding'));
  fireEvent.keyDown(item(container, 'layer:coding'), { key: 'Delete' }); await confirm();
  expect(draft().version.plan.nodes.map((n) => n.node_id)).toEqual(['test-a']);
  expect(draft().layout).toEqual({ testing: layout.testing });
  fireEvent.click(screen.getByText('整理布局'));
  expect(draft().layout).toEqual({});
});

it('历史预览的键盘不能删除节点', () => {
  const { container, rerender } = render(<PlanPreview plan={version.plan} capabilities={[capability]} />);
  const target = item(container, 'execution:code-a'); fireEvent.click(target);
  for (const keyName of ['Backspace', 'Delete']) fireEvent.keyDown(target, { key: keyName });
  rerender(<PlanPreview plan={{ ...version.plan }} capabilities={[capability]} />);
  expect(container.querySelectorAll('.react-flow__node-execution')).toHaveLength(3);
  expect(screen.queryByRole('dialog')).toBeNull();
});

it('删除层内最后一个节点保留空层草稿，提交时要求补齐节点', async () => {
  const { container } = mount();
  fireEvent.click(item(container, 'execution:test-a'));
  fireEvent.keyDown(item(container, 'execution:test-a'), { key: 'Delete' }); await confirm();
  expect(draft().version.plan.layers.map((layer) => layer.layer_id)).toEqual(['coding', 'testing']);
  fireEvent.click(screen.getByText('下一步：计划信息'));
  expect(screen.getByText('testing：每层需要 1–32 个并行执行节点')).toBeTruthy();
  expect(apiPost).not.toHaveBeenCalled();
});
