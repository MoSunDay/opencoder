// @vitest-environment jsdom
import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { ChatPanel } from '../chat.jsx';
import { setState } from '../store.js';
import { apiGet, apiPost } from '../api.js';
import { openStream } from '../sse.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn() }));

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/nodes') return { nodes: [{ id: 'n1', name: 'n1', online: true,
      kinds: ['operator', 'agent'], snapshot: { ready: true } }] };
    if (path.includes('/dialogs')) return { dialogs: [{ session_id: 's1', title: '等待中的会话' }] };
    if (path === '/api/agents') return { agents: [{ name: 'writer', primary: true }] };
    if (path === '/api/sessions/s1') return { initializing: true, draining: true, meta: { agent: 'writer' }, messages: [] };
    return {};
  });
});

it.each(['operator', 'agent'])('opens an accepted %s before its first event and subscribes for results', async (kind) => {
  localStorage.setItem('oc_chat_mode', JSON.stringify(kind));
  const { container } = render(<ChatPanel />);
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('n1', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(await screen.findByText('等待中的会话'));
  await waitFor(() => expect(openStream).toHaveBeenCalledOnce());
  const subscription = openStream.mock.calls[0][0];
  expect(subscription).toMatchObject({ sessionId: 's1', after: 0 });
  expect(container.querySelector('.ant-spin-spinning')).toBeTruthy();
  await act(async () => subscription.onFrame({ event: 'text_delta', data: { text: '远端已开始回答' } }));
  expect(await screen.findByText('远端已开始回答')).toBeTruthy();
  expect(screen.queryByText(/读取会话失败/)).toBeNull();
  expect(apiPost).not.toHaveBeenCalled();
});

it('reopens a running session with saved messages and resumes after the stored cursor', async () => {
  const original = apiGet.getMockImplementation();
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/sessions/s1/seq') return { seq: 24 };
    if (path === '/api/sessions/s1') return { draining: true, meta: { agent: 'writer' }, messages: [
      { role: 'user', blocks: [{ kind: 'text', text: '已有的需求' }] },
    ] };
    return original(path);
  });
  render(<ChatPanel />);
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('n1', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(await screen.findByText('等待中的会话'));
  await waitFor(() => expect(openStream).toHaveBeenCalledOnce());
  expect(openStream.mock.calls[0][0]).toMatchObject({ sessionId: 's1', after: 24 });
  expect(await screen.findByText('已有的需求')).toBeTruthy();
});

it('retries the same creation ID only while the first instruction stays the same', async () => {
  apiPost.mockRejectedValue(new Error('远端暂时不可用'));
  const { container } = render(<ChatPanel />);
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText('n1', { selector: '.ant-select-item-option-content' }));
  const input = container.querySelector('textarea.ant-sender-input');
  for (const [index, prompt] of ['原始需求', '原始需求', '修改后的需求'].entries()) {
    fireEvent.change(input, { target: { value: prompt } });
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
    await waitFor(() => expect(apiPost.mock.calls.filter(([path]) => path === '/api/sessions')).toHaveLength(index + 1));
    await waitFor(() => expect(input.value).toBe(prompt));
  }
  const requests = apiPost.mock.calls.filter(([path]) => path === '/api/sessions').map(([, body]) => body);
  expect(requests[1].id).toBe(requests[0].id);
  expect(requests[2].id).not.toBe(requests[0].id);
  expect(requests[2].prompt).toBe('修改后的需求');
});
