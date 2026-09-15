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
  apiPost.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValueOnce({ id: 'brain-created' }); const created = vi.fn();
  render(<Launch plans={[]} onCreated={created} />);
  fireEvent.change(screen.getByLabelText('目标和交付物'), { target: { value: '交付审核报告' } });
  fireEvent.mouseDown(screen.getByLabelText('大脑所在节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('节点 A', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByText('规划并执行')); await screen.findByText('connection lost');
  fireEvent.click(screen.getByText('规划并执行')); await waitFor(() => expect(created).toHaveBeenCalledWith('brain-created'));
  expect(apiPost.mock.calls[0][0]).toBe('/api/brain/runs'); expect(apiPost.mock.calls[0][1].id).toBe(apiPost.mock.calls[1][1].id);
  expect(apiPost.mock.calls[0][1]).toMatchObject({ mode: 'dynamic', plan: null, references: [] });
});
it('folds the optional prefill inputs behind advanced options and launches a one-liner without them', async () => {
  apiPost.mockResolvedValueOnce({ id: 'brain-one-liner' }); const created = vi.fn();
  render(<Launch plans={[]} onCreated={created} />);
  // 可选初始输入默认收进“高级选项”，主表单只见必填项。
  expect(screen.queryByLabelText(/初始输入/)).toBeNull();
  fireEvent.click(screen.getByText('高级选项'));
  expect(await screen.findByLabelText(/初始输入/)).toBeTruthy();
  expect(screen.getByPlaceholderText('留空即可，运行中会按需询问')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('目标和交付物'), { target: { value: '一句话发起，零预填输入' } });
  fireEvent.mouseDown(screen.getByLabelText('大脑所在节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('节点 A', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByText('规划并执行'));
  await waitFor(() => expect(created).toHaveBeenCalledWith('brain-one-liner'));
  expect(apiPost.mock.calls[0][1].inputs).toEqual({});
});
