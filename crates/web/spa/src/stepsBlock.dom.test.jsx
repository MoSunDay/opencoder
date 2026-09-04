// @vitest-environment jsdom
// StepsBlock DOM drill-down: Turn → Step → Function call. Zero clicks render
// the collapsed `❯ 2 Steps` (or, once the turn has its own Say, the
// `❯ Say(2 steps): {preview}` header) summary plus Say in ONE assistant
// bubble. Opening
// the Turn reveals Steps; opening a Step reveals Thinking + N Function calls;
// opening that aggregate reveals call rows, and a call reveals its result.
// Collapse-all works both ways: Ctrl+L on window and the `⤒ 收起`
// link bump the epoch key on Bubble.List, remounting every bubble so all
// (uncontrolled) Collapses reset closed.
// ./sse.js is module-mocked like subagentBlock.dom.test.jsx (transitively
// imported via subagentBlock.jsx; no drill-in is opened here).

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

const { openStreamMock } = vi.hoisted(() => ({ openStreamMock: vi.fn() }));
vi.mock('./sse.js', () => ({ openStream: openStreamMock }));
vi.mock('./api.js', () => ({
  apiGet: vi.fn().mockResolvedValue({}),
  apiPost: vi.fn().mockResolvedValue({}),
  apiDel: vi.fn().mockResolvedValue({}),
  signFetch: vi.fn(),
}));

import './test/setup-dom.js';
import { TranscriptView } from './transcript.jsx';

const stepsTurn = () => ({
  kind: 'steps',
  role: 'assistant',
  steps: [
    {
      thinking: 'look at the repo first',
      calls: [
        { kind: 'tool', role: 'assistant', id: 'a1', name: 'bash', input: 'ls -la', output: 'total 8', isError: false, durationMs: 1200, startedAt: 0 },
      ],
    },
    {
      thinking: '',
      calls: [
        { kind: 'tool', role: 'assistant', id: 'b1', name: 'read', input: 'src/main.rs', output: 'fn main() {}', isError: false, durationMs: 300, startedAt: 0 },
      ],
    },
  ],
});

const mount = (turns) => render(
  <TranscriptView turns={turns || [stepsTurn()]} usage={null} status="streaming" error={null} emptyText="无" />,
);

afterEach(() => {
  cleanup();
});

describe('StepsContent three-level drill-down', () => {
  it('renders ONLY the group row at zero clicks — the whole ladder stays out of the DOM', () => {
    mount();
    expect(screen.getByText('❯ 2 Steps')).toBeTruthy();
    // Step, Thinking, function-call rows and results are all hidden.
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText(/❯ Step\(2\)/)).toBeNull();
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText(/Function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('ls -la')).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('clicking the group row reveals the step rows; thinking stays hidden (L0 → L1)', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    expect(screen.getByText(/❯ Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/❯ Step\(2\)/)).toBeTruthy();
    // Not drilled into Step(1) yet: no thinking, no aggregate row, no calls.
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/Function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
  });

  it('clicking a step row shows thinking + calls aggregate, not call rows', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    expect(screen.getByText('💭 Thinking')).toBeTruthy();
    expect(screen.getByText('look at the repo first')).toBeTruthy();
    expect(screen.getByText('❯ 1 Function call')).toBeTruthy();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('ls -la')).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('clicking a single call row expands its exact input/output', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    fireEvent.click(screen.getByText('❯ 1 Function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('ls -la')).toBeTruthy();
    expect(screen.getByText('total 8')).toBeTruthy();
  });

  it('keeps user disclosure state when new output rerenders the turn', () => {
    const initial = stepsTurn();
    initial.steps[0].calls[0].output = null;
    const mounted = mount([initial]);
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    fireEvent.click(screen.getByText('❯ 1 Function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));

    const updated = stepsTurn();
    updated.steps[0].calls[0].output = 'new streamed output';
    mounted.rerender(
      <TranscriptView turns={[updated]} usage={null} status="streaming" error={null} emptyText="无" />,
    );

    expect(screen.getByText(/❯ Step\(1\)/)).toBeTruthy();
    expect(screen.getByText('❯ 1 Function call')).toBeTruthy();
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText('new streamed output')).toBeTruthy();

    fireEvent.click(screen.getByText('❯ 2 Steps'));
    expect(
      screen.getByText('❯ 2 Steps').closest('.ant-collapse-item').classList.contains('ant-collapse-item-active'),
    ).toBe(false);
    updated.steps[0].calls[0].output = 'later output';
    mounted.rerender(
      <TranscriptView turns={[updated]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    expect(
      screen.getByText('❯ 2 Steps').closest('.ant-collapse-item').classList.contains('ant-collapse-item-active'),
    ).toBe(false);
  });

  it('Ctrl+L collapses the fully-drilled ladder back to the lone group row', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    fireEvent.click(screen.getByText('❯ 1 Function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('total 8')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'l', ctrlKey: true });
    expect(screen.getByText('❯ 2 Steps')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('the ⤒ 收起 link resets the ladder the same way', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    fireEvent.click(screen.getByText(/❯ Step\(2\)/));
    fireEvent.click(screen.getByText('❯ 1 Function call'));
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
    fireEvent.click(screen.getByText('⤒ 收起'));
    expect(screen.getByText('❯ 2 Steps')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(2\)/)).toBeNull();
    expect(screen.queryByText(/🔧 read/)).toBeNull();
  });

  it('shows progress with a 12px gap even after calls finish, until Say starts', () => {
    const running = stepsTurn();
    running.progressActive = true;
    mount([running]);
    const tag = screen.getByText('running');
    expect(tag).toBeTruthy();
    expect(tag.style.marginLeft).toBe('12px');
    expect(screen.queryByText('error')).toBeNull();
  });

  it('a Say without sayActive retires the running tag and the header becomes the Say row', () => {
    // Even stale reducer state cannot show Running beside an existing Say:
    // once a Say streams in, the header switches to the Say form and the
    // running tag rides `sayActive` only (missing flag → no tag).
    const running = stepsTurn();
    running.progressActive = true;
    mount([running, { kind: 'text', role: 'assistant', text: 'Say started' }]);
    expect(screen.queryByText('running')).toBeNull();
    expect(screen.getByText('❯ Say(2 steps): Say started')).toBeTruthy();
    expect(screen.queryByText('❯ 2 Steps')).toBeNull();
  });

  it('keeps the running tag ON the Say row while sayActive (12px gap)', () => {
    const streaming = stepsTurn();
    mount([
      { ...streaming, sayStreaming: true },
      { kind: 'text', role: 'assistant', text: 'partial answer line one\nmore below' },
    ]);
    const tag = screen.getByText('running');
    expect(tag).toBeTruthy();
    expect(tag.style.marginLeft).toBe('12px');
    expect(screen.getByText('❯ Say(2 steps): partial answer line one')).toBeTruthy();
    expect(screen.queryByText('error')).toBeNull();
  });

  it('shows the Say-row error tag once running settles and a call failed', () => {
    const errored = stepsTurn();
    errored.steps[0].calls[0].isError = true;
    mount([errored, { kind: 'text', role: 'assistant', text: 'finished with a failure' }]);
    expect(screen.getByText('❯ Say(2 steps): finished with a failure')).toBeTruthy();
    expect(screen.getByText('error')).toBeTruthy();
    expect(screen.queryByText('running')).toBeNull();
  });

  it('stays collapsed while streaming so the default remains one Turn summary', () => {
    const streaming = {
      kind: 'steps',
      role: 'assistant',
      steps: [{ thinking: 'planning the next call', calls: [] }],
    };
    mount([streaming]);
    expect(screen.getByText('❯ 1 Step')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText('planning the next call')).toBeNull();
    cleanup();
    const openCall = stepsTurn();
    openCall.steps[0].calls[0].output = null;
    mount([openCall]);
    expect(screen.getByText('❯ 2 Steps')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
  });

  it('starts collapsed after streaming settles as well', () => {
    const settled = {
      kind: 'steps',
      role: 'assistant',
      steps: [{
        thinking: 'settled round',
        calls: [
          { kind: 'tool', role: 'assistant', id: 'z', name: 'bash', input: 'x', output: 'y', isError: false, durationMs: 5, startedAt: 0 },
        ],
      }],
    };
    mount([settled]);
    expect(screen.getByText('❯ 1 Step')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText('settled round')).toBeNull();
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText(/Function call/)).toBeNull();
  });

  it('shows the error tag on the group row (and on the failed step once expanded)', () => {
    const errored = stepsTurn();
    errored.steps[0].calls[0].isError = true;
    mount([errored]);
    // Zero clicks: only the group-row tag is in the document.
    expect(screen.getByText('error')).toBeTruthy();
    expect(screen.queryByText('running')).toBeNull();
    fireEvent.click(screen.getByText('❯ 2 Steps'));
    // Step(1) carries the failed call → its own red tag next to the label.
    expect(screen.getAllByText('error').length).toBe(2);
  });

  it('reveals every function-call row only after the aggregate opens', () => {
    const one = {
      kind: 'steps',
      role: 'assistant',
      steps: [{
        thinking: '',
        calls: [
          { kind: 'tool', role: 'assistant', id: 'a', name: 'bash', input: 'x', output: 'y', isError: false, durationMs: 10, startedAt: 0 },
          { kind: 'tool', role: 'assistant', id: 'b', name: 'read', input: 'x', output: 'y', isError: false, durationMs: 10, startedAt: 0 },
        ],
      }],
    };
    mount([one]);
    expect(screen.getByText('❯ 1 Step')).toBeTruthy();
    expect(screen.queryByText(/❯ 1 Steps/)).toBeNull();
    fireEvent.click(screen.getByText('❯ 1 Step'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    expect(screen.getByText('❯ 2 Function calls')).toBeTruthy();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText(/🔧 read/)).toBeNull();
    fireEvent.click(screen.getByText('❯ 2 Function calls'));
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
  });

  it('renders `N Steps + Say` as one visual Turn bubble', () => {
    const { container } = mount([
      stepsTurn(),
      { kind: 'text', role: 'assistant', text: 'all done here\nand the details follow' },
    ]);
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(1);
    // The Say merged INTO the header: label switches to the Say form with
    // the step count and the Say's first-line preview. The body below skips
    // that duplicated first line (③) — only the remaining lines render.
    const group = screen.getByText('❯ Say(2 steps): all done here');
    const say = screen.getByText('and the details follow');
    expect(group.compareDocumentPosition(say) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

// ② 头部行与正文的真实空行间距 + ③ 正文与头部 preview 的首行去重。
// 口径见 sayText.js：头部 preview 与被跳过的首行共用同一份拼接。
describe('Say body spacing & first-line dedup (② ③)', () => {
  const spacingWrap = (container) => Array.from(container.querySelectorAll('div'))
    .find((el) => el.style.marginTop === '16px');

  it('multi-line Say: body skips the preview-duplicated first line and keeps a 16px gap below the header', () => {
    const { container } = mount([
      stepsTurn(),
      { kind: 'text', role: 'assistant', text: 'line one\nline two\nline three' },
    ]);
    expect(screen.getByText('❯ Say(2 steps): line one')).toBeTruthy();
    // 首行已由头部渲染，正文不再重复。
    expect(screen.queryByText('line one')).toBeNull();
    // 其余行照常渲染（testing-library 会把换行归一化为空格）。
    expect(screen.getByText('line two line three')).toBeTruthy();
    // ② 真实空行间距：正文块 marginTop 16px（TUI 头部后插一空行的对齐）。
    const wrap = spacingWrap(container);
    expect(wrap).toBeTruthy();
    expect(wrap.textContent).toContain('line two');
  });

  it('single-line Say: no body block at all — no leftover spacing or empty node', () => {
    const { container } = mount([
      stepsTurn(),
      { kind: 'text', role: 'assistant', text: 'all done here' },
    ]);
    expect(screen.getByText('❯ Say(2 steps): all done here')).toBeTruthy();
    // 单行 Say 与 preview 一字不差 → 正文整块不渲染。
    expect(screen.queryByText('all done here')).toBeNull();
    // 无残留间距节点（没有任何 16px 的正文块包装）。
    expect(spacingWrap(container)).toBeUndefined();
    // Turn 泡内没有任何 Typography 段落 —— 不残留空文本块。
    const bubble = screen.getByText('❯ Say(2 steps): all done here').closest('.ant-bubble');
    expect(bubble.querySelectorAll('.ant-typography')).toHaveLength(0);
  });
});
