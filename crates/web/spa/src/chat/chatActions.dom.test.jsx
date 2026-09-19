// @vitest-environment jsdom
// Chat top bar + sidebar actions:
//   page-level 模式 Segmented (Operator/Agent) + Operator act/plan Segmented,
//   which is clickable once a node is selected — with no
//   session the choice is staged locally and rides session creation (`agent`
//   field); with an idle session it POSTs /agent. Sidebar rows expose a hover
//   删除 menu confirmed via Modal.confirm.
import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatPanel } from '../chat.jsx';
import { setState } from '../store.js';
import { apiDel, apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

const nodes = [{ id: 'n1', name: 'n1', online: true, kinds: ['agent', 'operator'], snapshot: { ready: true } }];
const dialogs = [{ session_id: 's1', title: '已有会话', first_created_at: 1, last_created_at: 2 }];
const snapshot = { meta: { agent: 'act' }, messages: [] };

const pick = async (name) => {
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText(name, { selector: '.ant-select-item-option-content' }));
};

const selectedSegment = (container, label) => container
  .querySelector(`.ant-segmented[aria-label="${label}"]`)
  ?.querySelector('.ant-segmented-item-selected')?.textContent;

const send = async (container, prompt) => {
  const input = container.querySelector('textarea.ant-sender-input');
  await waitFor(() => expect(input.disabled).toBe(false));
  fireEvent.change(input, { target: { value: prompt } });
  fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
};

beforeEach(() => {
  vi.resetAllMocks();
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/nodes') return { nodes };
    if (path.startsWith('/api/nodes/n1/dialogs')) return { dialogs };
    if (path === '/api/sessions/s1') return snapshot;
    if (path.endsWith('/seq')) return { seq: 0 };
    return {};
  });
  apiPost.mockImplementation(async (path) => path === '/api/sessions' ? { id: 'agent-created' } : { ok: true });
  apiDel.mockResolvedValue({ ok: true });
});

describe('chat top bar', () => {
  it('keeps act/plan + 模型 in the session controls with the 模式 Segmented ahead', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await waitFor(() => expect(container.querySelector('.ant-segmented')).toBeTruthy());
    // antd 6 inserts a space between two CJK button chars ("模 型").
    expect(screen.getByRole('button', { name: '模 型' })).toBeTruthy();
    // 页头模式 Segmented 常驻且缺省 Operator 模式（Agent 链路详见 chatMode 测试）。
    expect(container.querySelector('.ant-segmented[aria-label="会话模式"]')).toBeTruthy();
    expect(selectedSegment(container, '会话模式')).toBe('Operator 模式');
    expect(screen.queryByRole('button', { name: '知识追加' })).toBeNull();
    expect(screen.queryByRole('button', { name: '批注' })).toBeNull();
    expect(screen.queryByRole('button', { name: '压缩' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'autopilot' })).toBeNull();
  });

  it('act/plan is clickable before any session exists and stages the mode onto creation', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    const seg = await waitFor(() => {
      const el = container.querySelector('.ant-segmented[aria-label="agent 切换"] .ant-segmented-item-input');
      expect(el).toBeTruthy();
      expect(el.disabled).toBe(false);
      return el;
    });
    expect(seg.disabled).toBe(false);
    expect(selectedSegment(container, 'agent 切换')).toBe('act');

    await act(async () => {
      fireEvent.click(screen.getByText('plan'));
    });
    // Staged locally: no session, so no /agent POST yet.
    expect(selectedSegment(container, 'agent 切换')).toBe('plan');
    expect(apiPost).not.toHaveBeenCalled();

    await send(container, '只读规划');
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/sessions',
      { id: expect.stringMatching(/^operator-/), node_id: 'n1', agent: 'plan' }));
    expect(apiPost.mock.calls.filter(([path]) => path.endsWith('/agent'))).toHaveLength(0);
  });

  it('switches the mode of an opened session through POST /agent', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已有会话'));
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/sessions/s1'));
    expect(selectedSegment(container, 'agent 切换')).toBe('act');

    await act(async () => {
      fireEvent.click(screen.getByText('plan'));
    });
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/sessions/s1/agent', { value: 'plan' }));
    expect(selectedSegment(container, 'agent 切换')).toBe('plan');
  });
});

describe('sidebar session deletion', () => {
  it('deletes a session after the hover menu 删除 is confirmed', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已有会话'));
    await waitFor(() => expect(container.querySelector('li.ant-conversations-item-active')).toBeTruthy());

    // Hover reveals the ... trigger; the dropdown itself opens on click.
    fireEvent.click(container.querySelector('.ant-conversations-menu-icon'));
    const dropdown = await waitFor(() => {
      const el = document.querySelector('.ant-dropdown:not(.ant-dropdown-hidden)');
      expect(el).toBeTruthy();
      return el;
    });
    fireEvent.click(dropdown.querySelector('.ant-dropdown-menu-item'));

    // Confirm inside Modal.confirm
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal-confirm');
      expect(el).toBeTruthy();
      return el;
    });
    // Modal.confirm ok is the dangerous primary button.
    await act(async () => {
      fireEvent.click(modal.querySelector('.ant-btn-dangerous'));
    });
    await waitFor(() => expect(apiDel).toHaveBeenCalledWith('/api/sessions/s1'));
    await waitFor(() => {
      expect(container.querySelector('li.ant-conversations-item-active')).toBeNull();
      expect(screen.queryByText('已有会话')).toBeNull();
    });
  });

  it('keeps the session when the confirm dialog is cancelled', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已有会话'));
    await waitFor(() => expect(container.querySelector('li.ant-conversations-item-active')).toBeTruthy());

    fireEvent.click(container.querySelector('.ant-conversations-menu-icon'));
    const dropdown = await waitFor(() => {
      const el = document.querySelector('.ant-dropdown:not(.ant-dropdown-hidden)');
      expect(el).toBeTruthy();
      return el;
    });
    fireEvent.click(dropdown.querySelector('.ant-dropdown-menu-item'));
    const modal = await waitFor(() => {
      const el = document.querySelector('.ant-modal-confirm');
      expect(el).toBeTruthy();
      return el;
    });
    await act(async () => {
      const cancel = [...modal.querySelectorAll('.ant-modal-confirm-btns button')]
        .find((b) => !b.className.includes('ant-btn-dangerous'));
      fireEvent.click(cancel);
    });
    // antd keeps the closed dialog DOM around for its exit animation; poll
    // briefly and assert the delete was never issued.
    await act(async () => { await new Promise((r) => setTimeout(r, 120)); });
    expect(apiDel).not.toHaveBeenCalled();
    expect(await screen.findByText('已有会话')).toBeTruthy();
  });
});
