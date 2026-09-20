// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Launch } from '../launch.jsx';
import { apiGet, apiPost } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn(), apiGet: vi.fn() }));
vi.mock('../../../fleet/useNodes.js', () => ({ useNodes: () => ({ nodes: [{ id: 'node-a', name: '节点 A', online: true, kinds: ['brain'], snapshot: { ready: true } }] }) }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
const capabilities = [{ id: 'cap-a', kind: 'agent', target: 'act', summary: '审核', definition: {}, version: '1', input_desc: '请求', output_desc: '报告' }];
const node = async () => { fireEvent.mouseDown(screen.getByLabelText('大脑所在节点').closest('.ant-select')); fireEvent.click(await screen.findByText('节点 A', { selector: '.ant-select-item-option-content' })); };
it('uses v3 and run_id, preserving submission identity across an uncertain response', async () => {
  apiPost.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValueOnce({ schema_version: 3, run_id: 'brain-created' }); const created = vi.fn();
  render(<Launch capabilities={capabilities} onCreated={created} />);
  fireEvent.change(screen.getByLabelText('目标和交付物'), { target: { value: '交付审核报告' } });
  fireEvent.click(screen.getByRole('checkbox', { name: /审核/ })); await node();
  fireEvent.click(screen.getByText('开始执行')); await screen.findByText('connection lost');
  fireEvent.click(screen.getByText('开始执行')); await waitFor(() => expect(created).toHaveBeenCalledWith('brain-created'));
  expect(apiPost.mock.calls[0][1]).toEqual(apiPost.mock.calls[1][1]);
  expect(apiPost.mock.calls[0][1]).toMatchObject({ schema_version: 3, inputs: {}, capability_ids: ['cap-a'] });
  expect(apiPost.mock.calls[0][1]).not.toHaveProperty('mode');
});
it('starts an explicit saved version with engineering input overrides and no mutable plan fields', async () => {
  apiGet.mockResolvedValue({ id: 'plan-a', version: 2, plan: { schema_version: 3, title: '已保存', objective: 'verify', capability_ids: ['cap-a'], inputs: { repo: 'old' } } });
  apiPost.mockResolvedValue({ run_id: 'brain-reused' }); const created = vi.fn();
  render(<Launch capabilities={capabilities} initialPlan="plan-a@2" onCreated={created} />);
  await screen.findByText('已保存'); await node();
  fireEvent.change(screen.getByLabelText('工程参数值'), { target: { value: 'new-repo' } });
  fireEvent.click(screen.getByText('开始执行')); await waitFor(() => expect(created).toHaveBeenCalledWith('brain-reused'));
  expect(apiPost.mock.calls[0][1]).toMatchObject({ schema_version: 3, plan: { id: 'plan-a', version: 2 }, inputs: { repo: 'new-repo' } });
  expect(apiPost.mock.calls[0][1]).not.toHaveProperty('objective');
});
