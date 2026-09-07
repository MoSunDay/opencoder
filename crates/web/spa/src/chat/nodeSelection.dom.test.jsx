// @vitest-environment jsdom
import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatPanel } from '../chat.jsx';
import { setState } from '../store.js';
import { apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));
const nodes = ['n1', 'n2'].map((id) => ({ id, name: id, online: true, kinds: ['agent'], snapshot: { ready: true } }));
const pick = async (name) => {
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText(name, { selector: '.ant-select-item-option-content' }));
};
const send = async (container, prompt) => {
  const input = container.querySelector('textarea.ant-sender-input');
  await waitFor(() => expect(input.disabled).toBe(false));
  fireEvent.change(input, { target: { value: prompt } });
  fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
};
beforeEach(() => {
  vi.resetAllMocks();
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => path === '/api/nodes' ? { nodes } : path.endsWith('/seq') ? { seq: 0 } : { dialogs: [] });
  apiPost.mockImplementation(async (path) => path === '/api/sessions' ? { id: 'agent-created' } : { ok: true });
});

describe('explicit conversation node selection', () => {
  it('does not create or send until a node is selected, then pins creation to that node', async () => {
    const { container } = render(<ChatPanel />);
    const input = container.querySelector('textarea.ant-sender-input');
    expect(input.disabled).toBe(true);
    fireEvent.change(input, { target: { value: 'do work' } });
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
    expect(apiPost).not.toHaveBeenCalled();
    expect(apiGet).not.toHaveBeenCalledWith('/api/sessions?limit=50');
    await pick('n2');
    await send(container, 'do work');
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/sessions', { id: expect.stringMatching(/^agent-/), node_id: 'n2' }));
    expect(apiGet).toHaveBeenCalledWith('/api/nodes/n2/dialogs');
    expect(apiPost).toHaveBeenCalledWith('/api/sessions/agent-created/prompt', { prompt: 'do work', delivery: 'steer' });
  });

  it('preserves the chosen node, draft and request ID after uncertain creation', async () => {
    apiPost.mockRejectedValueOnce(new Error('connection lost'));
    const { container } = render(<ChatPanel />);
    await pick('n1'); await send(container, 'retry safely');
    await screen.findByText('error: connection lost');
    expect(container.querySelector('textarea.ant-sender-input').value).toBe('retry safely');
    await send(container, 'retry safely');
    await waitFor(() => expect(apiPost.mock.calls.filter(([path]) => path === '/api/sessions')).toHaveLength(2));
    const attempts = apiPost.mock.calls.filter(([path]) => path === '/api/sessions');
    expect(attempts[1][1]).toEqual(attempts[0][1]);
    expect(attempts[0][1].node_id).toBe('n1');
  });

  it('ignores a late conversation list from the previously selected node', async () => {
    let resolveFirst;
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/nodes') return { nodes };
      if (path === '/api/nodes/n1/dialogs') return new Promise((resolve) => { resolveFirst = resolve; });
      if (path === '/api/nodes/n2/dialogs') return { dialogs: [{ session_id: 'agent-b', title: 'node two conversation' }] };
      return {};
    });
    render(<ChatPanel />); await pick('n1'); await pick('n2');
    expect(await screen.findByText('node two conversation')).toBeTruthy();
    await act(async () => resolveFirst({ dialogs: [{ session_id: 'agent-a', title: 'late node one' }] }));
    expect(screen.queryByText('late node one')).toBeNull();
    expect(screen.getByText('node two conversation')).toBeTruthy();
  });

  it('disables offline nodes', async () => {
    apiGet.mockResolvedValueOnce({ nodes: [{ ...nodes[0], online: false }] });
    const { container } = render(<ChatPanel />);
    fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
    const option = await screen.findByText('n1', { selector: '.ant-select-item-option-content' });
    expect(option.closest('.ant-select-item-option').className).toContain('disabled');
    fireEvent.click(option);
    expect(container.querySelector('textarea.ant-sender-input').disabled).toBe(true);
    expect(apiPost).not.toHaveBeenCalled();
  });

  it('retains the draft and does not submit when the event cursor cannot be read', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/nodes') return { nodes };
      if (path.endsWith('/seq')) throw new Error('cursor unavailable');
      return { dialogs: [] };
    });
    const { container } = render(<ChatPanel />);
    await pick('n1'); await send(container, 'preserve this draft');
    await screen.findByText('error: cursor unavailable');
    expect(container.querySelector('textarea.ant-sender-input').value).toBe('preserve this draft');
    expect(apiPost.mock.calls.some(([path]) => path.endsWith('/prompt'))).toBe(false);
  });

  it('ignores a late history snapshot after selecting a different node', async () => {
    let resolveSnapshot;
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/nodes') return { nodes };
      if (path === '/api/nodes/n1/dialogs') return { dialogs: [{ session_id: 'agent-old', title: 'old conversation' }] };
      if (path === '/api/sessions/agent-old') return new Promise((resolve) => { resolveSnapshot = resolve; });
      return { dialogs: [] };
    });
    render(<ChatPanel />); await pick('n1');
    fireEvent.click(await screen.findByText('old conversation'));
    await waitFor(() => expect(resolveSnapshot).toBeTypeOf('function'));
    await pick('n2');
    await act(async () => resolveSnapshot({ messages: [{ role: 'assistant', content: 'stale history must stay hidden' }] }));
    expect(screen.queryByText('stale history must stay hidden')).toBeNull();
  });

  it('reads the model catalog from the selected conversation node', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/nodes') return { nodes };
      if (path === '/api/nodes/n2/dialogs') return { dialogs: [{ session_id: 'agent-two', title: 'selected conversation' }] };
      if (path === '/api/models?node_id=n2') return { models: ['node-two-model'], default: 'node-two-model' };
      return {};
    });
    render(<ChatPanel />); await pick('n2');
    fireEvent.click(await screen.findByText('selected conversation'));
    fireEvent.click(screen.getByRole('button', { name: /模\s*型/ }));
    expect(await screen.findByText('node-two-model')).toBeTruthy();
    expect(apiGet).not.toHaveBeenCalledWith('/api/models');
  });
});
