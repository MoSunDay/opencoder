// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Launch } from '../launch.jsx';
import { apiPost } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn(), apiGet: vi.fn() }));
vi.mock('../../../fleet/useNodes.js', () => ({ useNodes: () => ({ nodes: [{ id: 'node-a', name: '节点 A', online: true, kinds: ['brain'], snapshot: { ready: true } }] }) }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
it('keeps a durable run identity across an uncertain submission and exposes errors', async () => {
  apiPost.mockRejectedValueOnce(new Error('connection lost')).mockImplementationOnce(async (_path, body) => ({ schema_version: 3, run_id: body.id })); const created = vi.fn();
  render(<Launch plans={[]} onCreated={created} />);
  fireEvent.change(screen.getByLabelText('目标和交付物'), { target: { value: '交付审核报告' } });
  fireEvent.mouseDown(screen.getByLabelText('大脑所在节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('节点 A', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByText('开始调度')); await screen.findByText('connection lost');
  fireEvent.click(screen.getByText('开始调度')); await waitFor(() => expect(created).toHaveBeenCalledWith(apiPost.mock.calls[0][1].id));
  expect(apiPost.mock.calls[0][0]).toBe('/api/brain/runs'); expect(apiPost.mock.calls[0][1].id).toBe(apiPost.mock.calls[1][1].id);
  expect(apiPost.mock.calls[0][1]).toMatchObject({ schema_version: 3, capability_ids: [], max_rounds: 32, inputs: { request: '交付审核报告' } });
});
it('collects engineering inputs as a one-level KV list and launches a one-liner without them', async () => {
  apiPost.mockImplementationOnce(async (_path, body) => ({ schema_version: 3, run_id: body.id })); const created = vi.fn();
  render(<Launch plans={[]} onCreated={created} />);
  // 文档名称、input 名称与高级选项退场；工程描述默认零行，一句话即可发起。
  expect(screen.queryByLabelText('文档名称')).toBeNull();
  expect(screen.queryByLabelText('input 名称')).toBeNull();
  expect(screen.queryByText('高级选项')).toBeNull();
  fireEvent.click(screen.getByText('添加工程参数'));
  fireEvent.change(screen.getByLabelText('工程参数名'), { target: { value: ' repo ' } });
  fireEvent.change(screen.getByLabelText('工程参数值'), { target: { value: '3' } });
  fireEvent.click(screen.getByText('添加工程参数'));
  fireEvent.change(screen.getByLabelText('目标和交付物'), { target: { value: '一句话发起，工程参数一层 KV' } });
  fireEvent.mouseDown(screen.getByLabelText('大脑所在节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('节点 A', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByText('开始调度'));
  await waitFor(() => expect(created).toHaveBeenCalledWith(apiPost.mock.calls[0][1].id));
  // 空键行被忽略，值按 JSON 解析为数字。
  expect(apiPost.mock.calls[0][1].inputs).toEqual({ repo: 3 });
});
