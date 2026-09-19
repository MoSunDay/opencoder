// @vitest-environment jsdom
// chat 页「模式 Segmented」创建链路分流：
//   - Operator 模式（缺省）：newId('operator')，POST /api/sessions body 不带
//     kind / how_append（现状 wire 形状）；
//   - Agent 模式：newId('agent') + body.kind='agent' + first prompt + concrete agent；
//     首条需求随创建请求提交，并由 worker 追加到该 Agent 的 how；
//     节点下拉/可执行判定按 canUseNode(nodes, id, 'agent') 过滤；
//   - 模式经 usehooks-ts useLocalStorage 持久化（oc_chat_mode），陌生值收敛
//     回 Operator；
//   - Agent 模式不再渲染知识追加入口。
import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CHAT_MODE_STORAGE_KEY, ChatPanel, HOW_APPEND_MAX, howAppendBytes } from '../chat.jsx';
import { setState } from '../store.js';
import { apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

const nodes = [
  { id: 'n1', name: 'n1', online: true, kinds: ['agent', 'operator'], snapshot: { ready: true } },
  { id: 'n2', name: 'n2', online: true, kinds: ['operator'], snapshot: { ready: true } },
];

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

const modeGroup = (container) => container.querySelector('.ant-segmented[aria-label="会话模式"]');
const selectedMode = (container) => modeGroup(container)
  ?.querySelector('.ant-segmented-item-selected')?.textContent;
const switchMode = async (label) => {
  await act(async () => {
    fireEvent.click(screen.getByText(label));
  });
};

const createHits = () => apiPost.mock.calls.filter(([path]) => path === '/api/sessions');

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/nodes') return { nodes };
    if (path.startsWith('/api/nodes/n1/dialogs') || path.startsWith('/api/nodes/n2/dialogs')) return { dialogs: [] };
    if (path === '/api/agents') return { agents: [] };
    if (path.endsWith('/seq')) return { seq: 0 };
    return {};
  });
  apiPost.mockImplementation(async (path) => path === '/api/sessions' ? { id: 's1' } : { ok: true });
});

describe('chat mode Segmented (Operator / Agent)', () => {
  it('defaults to Operator 模式 and persists the picked mode into oc_chat_mode', async () => {
    const { container, unmount } = render(<ChatPanel />);
    expect(selectedMode(container)).toBe('Operator 模式');
    // 缺省不写入 localStorage（usehooks-ts 只在显式 set 时落盘）。
    expect(localStorage.getItem(CHAT_MODE_STORAGE_KEY)).toBeNull();

    await switchMode('Agent 模式');
    expect(selectedMode(container)).toBe('Agent 模式');
    expect(localStorage.getItem(CHAT_MODE_STORAGE_KEY)).toBe('"agent"');

    unmount();
    const { container: remounted } = render(<ChatPanel />);
    expect(selectedMode(remounted)).toBe('Agent 模式');
    expect(screen.getByLabelText('执行 Agent')).toBeTruthy();
  });

  it('falls back to Operator display for a corrupt stored value (never crashes)', async () => {
    localStorage.setItem(CHAT_MODE_STORAGE_KEY, '"bogus"');
    const { container } = render(<ChatPanel />);
    expect(selectedMode(container)).toBe('Operator 模式');
    expect(screen.queryByLabelText('执行 Agent')).toBeNull();
  });

  it('shows a concrete Agent selector in Agent mode and removes knowledge staging', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    expect(screen.queryByLabelText('执行 Agent')).toBeNull();
    await switchMode('Agent 模式');
    expect(await screen.findByLabelText('执行 Agent')).toBeTruthy();
    expect(screen.queryByText('知识追加')).toBeNull();
    expect(container.querySelector('.ant-segmented[aria-label="agent 切换"]')).toBeNull();
    expect(screen.getByRole('button', { name: '模 型' })).toBeTruthy();
  });

  it('reloads an independent dialog lane when switching between Operator and Agent', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/nodes') return { nodes };
      if (path.endsWith('/dialogs?kind=operator')) {
        return { dialogs: [{ session_id: 'operator-row', title: 'Operator 记录' }] };
      }
      if (path.endsWith('/dialogs?kind=agent')) {
        return { dialogs: [{ session_id: 'agent-row', title: 'Agent 记录' }] };
      }
      if (path === '/api/agents') return { agents: [] };
      return {};
    });
    const { container } = render(<ChatPanel />);
    await pick('n1');
    expect(await screen.findByText('Operator 记录')).toBeTruthy();
    await switchMode('Agent 模式');
    expect(await screen.findByText('Agent 记录')).toBeTruthy();
    expect(screen.queryByText('Operator 记录')).toBeNull();
    expect(apiGet).toHaveBeenCalledWith('/api/nodes/n1/dialogs?kind=operator');
    expect(apiGet).toHaveBeenCalledWith('/api/nodes/n1/dialogs?kind=agent');
    expect(container.querySelector('textarea.ant-sender-input')).toBeTruthy();
  });
});

describe('creation lanes', () => {
  it('Operator mode keeps the legacy body: no kind, no how_append', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await send(container, 'operator lane');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1]).toEqual({
      id: expect.stringMatching(/^operator-/), node_id: 'n1', agent: 'act',
    });
    expect(createHits()[0][1].kind).toBeUndefined();
    expect(createHits()[0][1].how_append).toBeUndefined();
  });

  it('Agent mode creates with kind=agent and the selected concrete Agent', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await switchMode('Agent 模式');

    await send(container, 'agent lane');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1]).toEqual({
      id: expect.stringMatching(/^agent-/),
      node_id: 'n1',
      agent: 'act',
      kind: 'agent',
      prompt: 'agent lane',
    });
  });

  it('switches the Agent selector before creation', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await switchMode('Agent 模式');
    const select = screen.getByLabelText('执行 Agent').closest('.ant-select');
    fireEvent.mouseDown(select);
    const plan = await waitFor(() => {
      const option = [...document.querySelectorAll('.ant-select-item-option')]
        .find((item) => item.textContent?.trim().startsWith('plan'));
      expect(option).toBeTruthy();
      return option;
    });
    fireEvent.click(plan);
    await send(container, 'plan this');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1]).toMatchObject({ agent: 'plan', kind: 'agent', prompt: 'plan this' });
  });
});

describe('how_append guard (8192 UTF-8 bytes)', () => {
  it('counts bytes not chars: 2730 CJK chars fit, 2731 breach the cap', () => {
    expect(howAppendBytes('中'.repeat(2730))).toBe(8190);
    expect(howAppendBytes('中'.repeat(2731))).toBe(8193);
    expect(HOW_APPEND_MAX).toBe(8192);
  });
});

describe('per-mode node executability', () => {
  it('filters node usability by the mode kind: operator-only node blocks Agent mode', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n2'); // kinds: ['operator']
    await send(container, 'operator ok');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1].id).toMatch(/^operator-/);

    await switchMode('Agent 模式');
    // n2 不支持 agent kind：nodeReady 翻 false → Sender 禁用，发送不可能发生。
    expect(await screen.findByText('所选节点当前不可执行，请选择可用节点')).toBeTruthy();
    expect(container.querySelector('textarea.ant-sender-input').disabled).toBe(true);
    expect(createHits()).toHaveLength(1);

    await pick('n1'); // kinds: ['agent', 'operator']
    await waitFor(() => expect(screen.queryByText('所选节点当前不可执行，请选择可用节点')).toBeNull());
    await send(container, 'agent ok');
    await waitFor(() => expect(createHits()).toHaveLength(2));
    expect(createHits()[1][1]).toMatchObject({ kind: 'agent' });
    expect(createHits()[1][1].id).toMatch(/^agent-/);
  });
});
