// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Plans } from '../plans.jsx';
import { apiGet, apiPost } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../../../store.js', () => ({ useStore: () => ({ identity: { name: 'tester' }, base: '' }) }));
vi.mock('../canvas.jsx', () => ({ PlanCanvas: ({ plan, positions, onPositions, onViewport, onSelect }) => <div aria-label="测试画布"><span>{JSON.stringify(positions)}</span>{plan.steps.map((s) => <button key={s.id} onClick={() => onSelect?.(s.id)}>{s.label}</button>)}<button onClick={() => { onPositions({ fix: { x: 111, y: 222 } }); onViewport({ x: 30, y: 40, zoom: 0.8 }); }}>移动画布节点</button></div> }));
const capabilities = [{ id: 'cap-act', kind: 'agent', target: 'act', summary: '修复执行实体' }];
let reload;
const mount = () => render(<Plans plans={[]} capabilities={capabilities} reload={reload} onRun={vi.fn()} />);
const open = () => fireEvent.click(screen.getByRole('button', { name: '新建计划' }));
const key = () => Object.keys(localStorage).find((k) => k.startsWith('oc:brain:plan-draft:'));
beforeEach(() => { localStorage.clear(); vi.resetAllMocks(); reload = vi.fn(); apiPost.mockResolvedValue({ id: 'saved' }); });
afterEach(cleanup);

it('opens a full-width right drawer and restores the canvas, action text and viewport before any submission', async () => {
  let view = mount(); open();
  const drawer = screen.getByRole('dialog', { name: '新建计划' });
  expect(drawer.closest('.ant-drawer').className).toContain('ant-drawer-right');
  expect(drawer.closest('.ant-drawer-content-wrapper').style.width).toBe('100%');
  expect(screen.queryByLabelText('计划名称')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '修复—复测—发布示例' }));
  fireEvent.change(screen.getByLabelText('要做什么'), { target: { value: '继续修复并提交证据' } });
  fireEvent.click(screen.getByText('移动画布节点'));
  const before = JSON.parse(localStorage.getItem(key()));
  expect(before.viewport.zoom).toBe(0.8);
  expect(before.version.plan.flow.transitions).toHaveLength(4);
  fireEvent.click(screen.getByRole('button', { name: '关闭画布' }));
  view.unmount(); view = mount(); open();
  expect(screen.getByLabelText('要做什么').value).toBe('继续修复并提交证据');
  expect(screen.getByLabelText('测试画布').textContent).toContain('111');
  expect(JSON.parse(localStorage.getItem(key())).version.id).toBe(before.version.id);
  expect(apiPost).not.toHaveBeenCalled();
  expect(apiGet).not.toHaveBeenCalled();
});

it('asks for name and summary only on submit, preserves routing, and clears the draft only after saving', async () => {
  mount(); open(); fireEvent.click(screen.getByRole('button', { name: '修复—复测—发布示例' }));
  const cacheKey = key();
  fireEvent.click(screen.getByRole('button', { name: '提交计划' }));
  expect(apiPost).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: '问题修复' } });
  fireEvent.change(screen.getByLabelText('一句话概述'), { target: { value: '复测发现问题就回退，全部通过再发布' } });
  fireEvent.click(screen.getByRole('button', { name: '确认提交' }));
  await waitFor(() => expect(reload).toHaveBeenCalledOnce());
  const value = apiPost.mock.calls.find(([path]) => path === '/api/brain/plan-defs')[1];
  expect(value.plan).toMatchObject({ title: '问题修复', objective: '复测发现问题就回退，全部通过再发布', flow: { entry: 'fix' } });
  expect(value.plan.flow.transitions[1]).toMatchObject({ from: 'verify', to: 'fix', when: { equals: false } });
  expect(value.plan.steps.every((s) => s.capability_id === 'cap-act')).toBe(true);
  expect(localStorage.getItem(cacheKey)).toBeNull();
});

it('keeps failed submissions in browser storage and retries the same plan id after a page reload', async () => {
  apiPost.mockImplementation(async (path) => { if (path.endsWith('/validate')) return {}; throw new Error('保存失败'); });
  let view = mount(); open(); fireEvent.click(screen.getByRole('button', { name: '修复—复测—发布示例' }));
  fireEvent.click(screen.getByRole('button', { name: '提交计划' }));
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: '待重试计划' } });
  fireEvent.change(screen.getByLabelText('一句话概述'), { target: { value: '确保复测通过' } });
  fireEvent.click(screen.getByRole('button', { name: '确认提交' }));
  await screen.findByText('保存失败');
  const id = JSON.parse(localStorage.getItem(key())).version.id;
  view.unmount(); view = mount(); open();
  fireEvent.click(screen.getByRole('button', { name: '提交计划' }));
  expect(screen.getByLabelText('计划名称').value).toBe('待重试计划');
  apiPost.mockResolvedValue({ id });
  fireEvent.click(screen.getByRole('button', { name: '确认提交' }));
  await waitFor(() => expect(reload).toHaveBeenCalledOnce());
  expect(apiPost.mock.calls.filter(([path]) => path === '/api/brain/plan-defs').map(([, value]) => value.id)).toEqual([id, id]);
});

it('retains invalid contract text across reloads and never submits the last valid plan silently', async () => {
  let view = mount(); open(); fireEvent.click(screen.getByRole('button', { name: '修复—复测—发布示例' }));
  fireEvent.click(screen.getByText('高级契约'));
  fireEvent.change(screen.getByLabelText('输入端口与来源绑定'), { target: { value: '{broken' } });
  view.unmount(); view = mount(); open();
  fireEvent.click(screen.getByText('高级契约'));
  expect(screen.getByLabelText('输入端口与来源绑定').value).toBe('{broken');
  expect(screen.getByRole('button', { name: '提交计划' }).disabled).toBe(true);
  expect(apiPost).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('输入端口与来源绑定'), { target: { value: '{}' } });
  await waitFor(() => expect(screen.getByRole('button', { name: '提交计划' }).disabled).toBe(false));
});

it('surfaces storage failures and does not close or submit an uncached draft', async () => {
  mount(); open();
  const write = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('quota full'); });
  fireEvent.click(screen.getByRole('button', { name: '修复—复测—发布示例' }));
  expect(await screen.findByText('草稿未写入浏览器：quota full')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '关闭画布' }));
  expect(screen.getByRole('dialog', { name: '新建计划' })).toBeTruthy();
  expect(screen.getByRole('button', { name: '提交计划' }).disabled).toBe(true);
  write.mockRestore();
  fireEvent.click(screen.getByRole('button', { name: '重试缓存' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '提交计划' }).disabled).toBe(false));
}, 15000);
