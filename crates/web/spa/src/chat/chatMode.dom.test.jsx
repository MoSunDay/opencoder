// @vitest-environment jsdom
// chat 页「模式 Segmented」创建链路分流：
//   - Operator 模式（缺省）：newId('operator')，POST /api/sessions body 不带
//     kind / how_append（现状 wire 形状）；
//   - Agent 模式：newId('agent') + body.kind='agent' + staged how_append；
//     节点下拉/可执行判定按 canUseNode(nodes, id, 'agent') 过滤；
//   - 模式经 usehooks-ts useLocalStorage 持久化（oc_chat_mode），陌生值收敛
//     回 Operator；
//   - 知识追加 how_append 超 8192 字节（UTF-8 字节，非字符数）被双重拦截：
//     弹窗禁止保存 + 创建前 throw（send 恢复草稿、不发起创建 POST）。
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

const stageHowAppend = async (text) => {
  fireEvent.click(await screen.findByRole('button', { name: /知识追加/ }));
  const area = await screen.findByLabelText('知识追加内容');
  fireEvent.change(area, { target: { value: text } });
  fireEvent.click(screen.getByRole('button', { name: /保\s*存/ }));
  // 暂存回显：入口按钮带上「已暂存」标记（弹窗 DOM 会随退场动画驻留，不以其
  // 消失为信号）。
  await waitFor(() => expect(screen.getByRole('button', { name: '知识追加 · 已暂存' })).toBeTruthy());
};

const createHits = () => apiPost.mock.calls.filter(([path]) => path === '/api/sessions');

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  setState({ preselectNode: null, nodes: [] });
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/nodes') return { nodes };
    if (path === '/api/nodes/n1/dialogs' || path === '/api/nodes/n2/dialogs') return { dialogs: [] };
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
    expect(screen.getByRole('button', { name: /知识追加/ })).toBeTruthy();
  });

  it('falls back to Operator display for a corrupt stored value (never crashes)', async () => {
    localStorage.setItem(CHAT_MODE_STORAGE_KEY, '"bogus"');
    const { container } = render(<ChatPanel />);
    expect(selectedMode(container)).toBe('Operator 模式');
    expect(screen.queryByRole('button', { name: /知识追加/ })).toBeNull();
  });

  it('hides the 知识追加 entry in Operator mode and shows it in Agent mode', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    expect(screen.queryByRole('button', { name: /知识追加/ })).toBeNull();
    await switchMode('Agent 模式');
    expect(await screen.findByRole('button', { name: /知识追加/ })).toBeTruthy();
    // 会话级控制（act/plan、模型）在两种模式下都还在。
    expect(container.querySelector('.ant-segmented[aria-label="agent 切换"]')).toBeTruthy();
    expect(screen.getByRole('button', { name: '模 型' })).toBeTruthy();
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

  it('Agent mode creates with kind=agent, agent- id and the staged how_append', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await switchMode('Agent 模式');
    await stageHowAppend('仓库约定：回归前先跑 vitest');
    // 暂存回显在入口按钮上。
    expect(screen.getByRole('button', { name: '知识追加 · 已暂存' })).toBeTruthy();

    await send(container, 'agent lane');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1]).toEqual({
      id: expect.stringMatching(/^agent-/),
      node_id: 'n1',
      agent: 'act',
      kind: 'agent',
      how_append: '仓库约定：回归前先跑 vitest',
    });
  });

  it('Agent mode without staged knowledge omits how_append but still sends kind=agent', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await switchMode('Agent 模式');
    await send(container, 'no addendum');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1]).toEqual({
      id: expect.stringMatching(/^agent-/), node_id: 'n1', agent: 'act', kind: 'agent',
    });
    expect(createHits()[0][1].how_append).toBeUndefined();
  });
});

describe('how_append guard (8192 UTF-8 bytes)', () => {
  it('counts bytes not chars: 2730 CJK chars fit, 2731 breach the cap', () => {
    expect(howAppendBytes('中'.repeat(2730))).toBe(8190);
    expect(howAppendBytes('中'.repeat(2731))).toBe(8193);
    expect(HOW_APPEND_MAX).toBe(8192);
  });

  it('shows the byte error, blocks saving, and keeps the over-limit text off the wire', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await switchMode('Agent 模式');
    fireEvent.click(await screen.findByRole('button', { name: /知识追加/ }));

    const over = '中'.repeat(2731); // 8193 字节 > 8192（字符数 2731 < 8192，证明按字节计）
    fireEvent.change(await screen.findByLabelText('知识追加内容'), { target: { value: over } });
    expect(await screen.findByText(/8193 \/ 8192 字节/)).toBeTruthy();
    expect(await screen.findByText(/超过上限，无法保存/)).toBeTruthy();
    const save = screen.getByRole('button', { name: /保\s*存/ });
    expect(save.disabled).toBe(true);

    // 关闭弹窗（保存被禁 → 未暂存任何内容），入口不显示已暂存。
    fireEvent.click(screen.getByRole('button', { name: /取\s*消/ }));
    await waitFor(() => expect(screen.queryByRole('button', { name: /已暂存/ })).toBeNull());

    // 超限内容无法经 UI 暂存 → 随创建提交的 body 永远不含 how_append。
    await send(container, 'must not carry addendum');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1].kind).toBe('agent');
    expect(createHits()[0][1].how_append).toBeUndefined();
  });

  it('clears the staged addendum via 清空 (empty save clears the entry)', async () => {
    const { container } = render(<ChatPanel />);
    await pick('n1');
    await switchMode('Agent 模式');
    await stageHowAppend('先暂存一段知识');
    fireEvent.click(await screen.findByRole('button', { name: /知识追加/ }));
    fireEvent.click(screen.getByRole('button', { name: /清\s*空/ }));
    await waitFor(() => expect(screen.queryByRole('button', { name: /已暂存/ })).toBeNull());

    await send(container, 'after clear');
    await waitFor(() => expect(createHits()).toHaveLength(1));
    expect(createHits()[0][1].how_append).toBeUndefined();
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
