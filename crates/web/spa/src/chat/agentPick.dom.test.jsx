// @vitest-environment jsdom
// Composer `@` agent picker + `/agent` command entry (TUI /agent parity):
//   `@` opens the fuzzy agent menu (builtin primary roles merged ahead of the
//   GET /api/agents cards, non-primary cards never listed);
//   picking an agent with an idle session POSTs /api/sessions/:id/agent,
//   with no session it stages onto creation (`agent` field), and while a
//   drain runs it travels as the `/agent <name>` TEXT head via /prompt steer;
//   the `/agent` command entry only completes the token (manual name mode).
import '../test/setup-dom.js';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatPanel } from '../chat.jsx';
import { setState } from '../store.js';
import { apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

const nodes = [{ id: 'n1', name: 'n1', online: true, kinds: ['agent', 'operator'], snapshot: { ready: true } }];
const dialogs = [{ session_id: 's1', title: '已有会话', first_created_at: 1, last_created_at: 2 }];
const snapshot = { meta: { agent: 'act' }, messages: [] };
const agents = [
  { name: 'writer', description: 'Writer soul: small diffs.', primary: true },
  { name: 'ghost', description: '非 primary 不可选', primary: false },
];

const pick = async (name) => {
  fireEvent.mouseDown(screen.getByLabelText('执行节点').closest('.ant-select'));
  fireEvent.click(await screen.findByText(name, { selector: '.ant-select-item-option-content' }));
};

const type = async (container, text) => {
  const input = container.querySelector('textarea.ant-sender-input');
  await waitFor(() => expect(input.disabled).toBe(false));
  fireEvent.change(input, { target: { value: text } });
  return input;
};

const clickRow = (container, cmd) => {
  const row = container.querySelector(`[data-cmd="${cmd}"]`);
  expect(row).toBeTruthy();
  fireEvent.click(row);
};

beforeEach(() => {
  vi.resetAllMocks();
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/nodes') return { nodes };
    if (path === '/api/nodes/n1/dialogs') return { dialogs };
    if (path === '/api/sessions/s1') return snapshot;
    if (path === '/api/agents') return { agents };
    if (path.endsWith('/seq')) return { seq: 0 };
    return {};
  });
  apiPost.mockImplementation(async (path) => path === '/api/sessions' ? { id: 'agent-created' } : { ok: true });
});

describe('composer @ agent menu', () => {
  it('fuzzy-lists builtin primary roles first, then registered primary cards', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await type(container, '@');
    // Builtin act/plan/command merge in front of the registered writer card;
    // the non-primary ghost card is never selectable.
    const cmds = [...container.querySelectorAll('[data-cmd]')].map((el) => el.dataset.cmd);
    expect(cmds).toEqual(['@act', '@plan', '@command', '@writer']);
    // The registered card carries its server-computed description.
    expect(container.querySelector('[data-cmd="@writer"]').textContent)
      .toContain('Writer soul: small diffs.');
  });

  it('fuzzy-matches registered agent names and lists nothing on a miss', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await type(container, '@wr');
    expect(container.querySelector('[data-cmd="@writer"]')).toBeTruthy();
    await type(container, '@zz');
    expect(container.querySelector('[data-cmd]')).toBeNull();
  });

  it('renders no menu before an execution node is selected', () => {
    const { container } = render(<ChatPanel />);
    const input = container.querySelector('textarea.ant-sender-input');
    expect(input.disabled).toBe(true);
    fireEvent.change(input, { target: { value: '@wr' } });
    expect(container.querySelector('[data-cmd]')).toBeNull();
  });
});

describe('agent pick', () => {
  it('switches an opened idle session via POST /agent and clears the token', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已有会话'));
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/sessions/s1'));

    await type(container, '@wr');
    clickRow(container, '@writer');
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/sessions/s1/agent', { value: 'writer' }));
    // The @ token is stripped from the composer, nothing else is sent.
    expect(container.querySelector('textarea.ant-sender-input').value).toBe('');
    expect(apiPost.mock.calls.filter(([p]) => p.endsWith('/prompt'))).toHaveLength(0);
  });

  it('stages the pick onto session creation when no session exists', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');

    await type(container, '@wr');
    clickRow(container, '@writer');
    // Staged locally: no /agent POST without a session.
    await waitFor(() => expect(apiPost).not.toHaveBeenCalled());

    const input = container.querySelector('textarea.ant-sender-input');
    fireEvent.change(input, { target: { value: '开工' } });
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/sessions',
      { id: expect.stringMatching(/^operator-/), node_id: 'n1', agent: 'writer' }));
    expect(apiPost.mock.calls.filter(([p]) => p.endsWith('/agent'))).toHaveLength(0);
  });

  it('steers a running drain with the /agent <name> text head', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已有会话'));
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/sessions/s1'));

    // First prompt starts the drain: busy latches (stream stays open).
    const input = container.querySelector('textarea.ant-sender-input');
    fireEvent.change(input, { target: { value: '第一条' } });
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/sessions/s1/prompt',
      { prompt: '第一条', delivery: 'steer' }));

    await type(container, '@wr');
    clickRow(container, '@writer');
    await waitFor(() => expect(apiPost).toHaveBeenLastCalledWith('/api/sessions/s1/prompt',
      { prompt: '/agent writer', delivery: 'steer' }));
    expect(container.querySelector('textarea.ant-sender-input').value).toBe('');
  });
});

describe('/agent command entry', () => {
  it('only completes the token for a manual name and never executes', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    fireEvent.click(await screen.findByText('已有会话'));
    await waitFor(() => expect(apiGet).toHaveBeenCalledWith('/api/sessions/s1'));

    await type(container, '/age');
    expect(container.querySelector('[data-cmd="/agent"]')).toBeTruthy();
    clickRow(container, '/agent');
    await waitFor(() => expect(container.querySelector('textarea.ant-sender-input').value).toBe('/agent '));
    // Completion only — the switch POST is the runner's job once the name
    // rides the prompt.
    expect(apiPost).not.toHaveBeenCalled();
  });
});
