// @vitest-environment jsdom
// Sidebar bottom 「删除全部会话」: one-click bulk clear behind a Modal.confirm
// gate. Contract: DELETE /api/nodes/:id/dialogs removes every terminal dialog
// of the selected node while the server SKIPS sessions whose node task is
// still pending/running/cancelling — the refreshed list keeps those. Button
// is disabled without a picked node and on a node without dialogs.
import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatPanel } from '../chat.jsx';
import { setState } from '../store.js';
import { apiDel, apiGet } from '../api.js';

vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

const nodes = [
  { id: 'n1', name: 'n1', online: true, kinds: ['agent', 'operator'], snapshot: { ready: true } },
  { id: 'n2', name: 'n2', online: true, kinds: ['agent', 'operator'], snapshot: { ready: true } },
];
const allDialogs = [
  { session_id: 's-done', title: '已完成会话', first_created_at: 1, last_created_at: 4 },
  { session_id: 's-run', title: '运行中会话', first_created_at: 2, last_created_at: 5 },
];
const snapshot = { meta: { agent: 'act' }, messages: [] };

/// Session ids that survive the sweep — the server keeps non-terminal ones.
let remaining = allDialogs.map((d) => d.session_id);
let emptyNode = false;

const pick = async (name) => {
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText(name, { selector: '.ant-select-item-option-content' }));
};

const clearButton = () => screen.getByRole('button', { name: /删除全部会话/ });

/// antd's static Modal.confirm portals to document.body and keeps closed
/// dialogs around for their exit animation, so leftovers from earlier cases
/// can linger. Wait until a NEW confirm container appears, then drive it.
const confirmModal = async () => {
  const before = document.querySelectorAll('.ant-modal-confirm').length;
  await act(async () => { fireEvent.click(clearButton()); });
  return waitFor(() => {
    const els = document.querySelectorAll('.ant-modal-confirm');
    expect(els.length).toBeGreaterThan(before);
    return els[els.length - 1];
  });
};

beforeEach(() => {
  vi.resetAllMocks();
  remaining = allDialogs.map((d) => d.session_id);
  emptyNode = false;
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/nodes') return { nodes };
    if (path === '/api/nodes/n1/dialogs') {
      return { dialogs: emptyNode ? [] : allDialogs.filter((d) => remaining.includes(d.session_id)) };
    }
    if (path.startsWith('/api/sessions/')) return snapshot;
    if (path.endsWith('/seq')) return { seq: 0 };
    return {};
  });
  apiDel.mockImplementation(async (path) => {
    if (path === '/api/nodes/n1/dialogs') {
      remaining = ['s-run'];
      return { ok: true, removed: 1, skipped: ['s-run'] };
    }
    return { ok: true };
  });
});

describe('chat sidebar 删除全部会话', () => {
  it('confirms, calls the node bulk endpoint, and keeps running dialogs listed', async () => {
    const onNotice = vi.fn();
    render(<ChatPanel onNotice={onNotice} />);
    await pick('n1');
    await waitFor(() => expect(screen.getByText('已完成会话')).toBeTruthy());

    const modal = await confirmModal();
    expect(modal.textContent).toContain('正在运行中的会话会保留');

    await act(async () => { fireEvent.click(modal.querySelector('.ant-btn-dangerous')); });
    await waitFor(() => expect(apiDel).toHaveBeenCalledWith('/api/nodes/n1/dialogs'));
    // List reloads from the server: the running dialog survives, the
    // terminal one is gone.
    await waitFor(() => expect(screen.queryByText('已完成会话')).toBeNull());
    expect(await screen.findByText('运行中会话')).toBeTruthy();
    expect(onNotice).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'success', text: '已删除 1 个会话，1 个运行中的会话已保留' }),
    );
  });

  it('clears the active selection when the open dialog is swept away', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已完成会话'));
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/sessions/s-done'));

    const modal = await confirmModal();
    await act(async () => { fireEvent.click(modal.querySelector('.ant-btn-dangerous')); });
    // The swept-away dialog must not stay highlighted; the surviving item
    // may become the list's new first-entry highlight, but s-done is gone.
    await waitFor(() => {
      const active = container.querySelector('li.ant-conversations-item-active');
      expect(active?.querySelector('.ant-conversations-label')?.textContent).not.toBe('已完成会话');
    });
    expect(screen.queryByText('已完成会话')).toBeNull();
  });

  it('is disabled before a node is picked and on a node without dialogs', async () => {
    render(<ChatPanel />);
    await waitFor(() => expect(clearButton().disabled).toBe(true));

    await pick('n1');
    await waitFor(() => expect(screen.getByText('已完成会话')).toBeTruthy());
    expect(clearButton().disabled).toBe(false);

    // Empty node: nothing to sweep, button goes back to disabled.
    emptyNode = true;
    await pick('n2');
    await waitFor(() => expect(clearButton().disabled).toBe(true));
    expect(apiDel).not.toHaveBeenCalled();
  });
});
