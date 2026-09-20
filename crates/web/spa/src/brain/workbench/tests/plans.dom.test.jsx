// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Plans } from '../plans.jsx';
import { apiGet, apiPost } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../../../store.js', () => ({ useStore: () => ({ identity: { name: 'tester' }, base: '' }) }));
const capabilities = [{ id: 'cap-act', kind: 'agent', target: 'act', summary: '修复执行实体', input_desc: '请求', output_desc: '报告', definition: {}, version: '1' }];
let reload;
const mount = (plans = []) => render(<Plans plans={plans} capabilities={capabilities} reload={reload} onRun={vi.fn()} />);
const open = () => fireEvent.click(screen.getByRole('button', { name: '新建计划' }));
const key = () => Object.keys(localStorage).find((k) => k.startsWith('oc:brain:scheduler-draft:v3:'));
const fill = () => {
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: '修复并复测' } });
  fireEvent.change(screen.getByLabelText('目标和交付物'), { target: { value: '交付修复及测试证据' } });
  fireEvent.click(screen.getByRole('checkbox', { name: /修复执行实体/ }));
};
beforeEach(() => { localStorage.clear(); vi.resetAllMocks(); reload = vi.fn(); apiPost.mockResolvedValue({ id: 'saved' }); });
afterEach(cleanup);

it('restores the simple draft and selected capabilities without touching old graph caches', () => {
  const old = '{old-v2-draft}'; localStorage.setItem('oc:brain:plan-draft:old:new', old);
  let view = mount(); open(); fill();
  expect(screen.getByLabelText('大脑调度总览画布')).toBeTruthy();
  expect(screen.queryByText('添加路由')).toBeNull();
  const id = JSON.parse(localStorage.getItem(key())).version.id;
  fireEvent.click(screen.getByRole('button', { name: '关闭画布' }));
  view.unmount(); view = mount(); open();
  expect(screen.getByLabelText('计划名称').value).toBe('修复并复测');
  expect(screen.getByRole('checkbox', { name: /修复执行实体/ }).checked).toBe(true);
  expect(JSON.parse(localStorage.getItem(key())).version.id).toBe(id);
  expect(localStorage.getItem('oc:brain:plan-draft:old:new')).toBe(old);
  expect(apiPost).not.toHaveBeenCalled();
});
it('saves a reusable scheduler plan and only clears the draft after acknowledgement', async () => {
  mount(); open(); fill(); const cacheKey = key();
  fireEvent.click(screen.getByRole('button', { name: '保存计划' }));
  await waitFor(() => expect(reload).toHaveBeenCalledOnce());
  const body = apiPost.mock.calls.find(([path]) => path === '/api/brain/plan-defs')[1];
  expect(body.plan).toEqual({ schema_version: 3, title: '修复并复测', objective: '交付修复及测试证据', inputs: {}, capability_ids: ['cap-act'], max_rounds: 32 });
  expect(localStorage.getItem(cacheKey)).toBeNull();
});
it('retains the exact plan identity and timestamp after uncertain submission and reload', async () => {
  apiPost.mockImplementation(async (path) => { if (path.endsWith('/validate')) return {}; throw new Error('保存失败'); });
  let view = mount(); open(); fill(); fireEvent.click(screen.getByRole('button', { name: '保存计划' }));
  await screen.findByText('保存失败'); const original = apiPost.mock.calls.find(([path]) => path === '/api/brain/plan-defs')[1];
  view.unmount(); view = mount(); open(); apiPost.mockResolvedValue({});
  fireEvent.click(screen.getByRole('button', { name: '保存计划' }));
  await waitFor(() => expect(reload).toHaveBeenCalledOnce());
  expect(apiPost.mock.calls.filter(([path]) => path === '/api/brain/plan-defs').at(-1)[1]).toEqual(original);
});
it('does not expose execution or edit actions for historical plans', () => {
  mount([{ id: 'old', title: '历史图', schema_version: 2, latest_version: 1 }]);
  expect(screen.getByText('历史只读')).toBeTruthy();
  expect(screen.queryByRole('button', { name: '执行' })).toBeNull();
  expect(screen.queryByRole('button', { name: '创建下一版本' })).toBeNull();
});
it('does not close or submit when browser storage is unavailable', async () => {
  mount(); open(); const write = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('quota full'); });
  fireEvent.change(screen.getByLabelText('计划名称'), { target: { value: 'keep me' } });
  expect(await screen.findByText('草稿未写入浏览器：quota full')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '关闭画布' }));
  expect(screen.getByRole('dialog', { name: '新建计划' })).toBeTruthy();
  expect(screen.getByRole('button', { name: '保存计划' }).disabled).toBe(true);
  write.mockRestore(); fireEvent.click(screen.getByRole('button', { name: '重试缓存' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '保存计划' }).disabled).toBe(false));
});
