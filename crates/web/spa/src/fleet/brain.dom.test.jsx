// @vitest-environment jsdom
import '../test/setup-dom.js';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BrainDispatch } from './brain.jsx';
import { apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn() }));
vi.mock('./detail.jsx', () => ({ ExecutionDetail: ({ id }) => <div>execution:{id}</div> }));
const nodes = ['n1', 'n2'].map((id) => ({ id, name: id, online: true, kinds: ['agent'], snapshot: { ready: true } }));
const pick = async (name) => {
  fireEvent.mouseDown(screen.getByLabelText('目标节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText(name, { selector: '.ant-select-item-option-content' }));
};
beforeEach(() => {
  vi.resetAllMocks(); apiGet.mockResolvedValue({ nodes });
  apiPost.mockResolvedValue({ execution: { id: 'agent-result' } });
});
describe('Brain request execution', () => {
  it('requires a chosen node and nonblank request before starting', async () => {
    render(<BrainDispatch />);
    fireEvent.change(screen.getByLabelText('需求'), { target: { value: 'do work' } });
    fireEvent.click(screen.getByText('开始执行'));
    await waitFor(() => expect(screen.getAllByText('请选择目标节点').length).toBeGreaterThan(1));
    expect(apiPost).not.toHaveBeenCalled();
    await pick('n1');
    fireEvent.change(screen.getByLabelText('需求'), { target: { value: '   ' } });
    fireEvent.click(screen.getByText('开始执行'));
    expect(await screen.findByText('请输入需求')).toBeTruthy();
    expect(apiPost).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText('需求'), { target: { value: 'finish work' } });
    fireEvent.click(screen.getByText('开始执行'));
    expect(await screen.findByText('execution:agent-result')).toBeTruthy();
    expect(apiPost).toHaveBeenCalledWith('/api/brain/dispatch', { situation: 'finish work', node_id: 'n1', request_id: expect.stringMatching(/^request-/) });
    expect(apiGet).not.toHaveBeenCalledWith('/api/brain/capabilities');
    expect(screen.queryByText('预览路由')).toBeNull();
    expect(screen.queryByText('绑定能力执行目标')).toBeNull();
  });

  it('uses a new request ID after an unconfirmed request changes its target node', async () => {
    apiPost.mockRejectedValue(new Error('connection lost'));
    render(<BrainDispatch />); await pick('n1');
    fireEvent.change(screen.getByLabelText('需求'), { target: { value: 'do work' } });
    fireEvent.click(screen.getByText('开始执行'));
    await screen.findByText('connection lost');
    await pick('n2'); fireEvent.click(screen.getByText('开始执行'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(2));
    expect(apiPost.mock.calls[0][1].node_id).toBe('n1');
    expect(apiPost.mock.calls[1][1].node_id).toBe('n2');
    expect(apiPost.mock.calls[0][1].request_id).not.toBe(apiPost.mock.calls[1][1].request_id);
  });
});
