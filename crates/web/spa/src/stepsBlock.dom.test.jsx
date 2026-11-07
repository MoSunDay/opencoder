// @vitest-environment jsdom
// StepsBlock DOM drill-down: Turn → Step → Function call. A SETTLED turn
// renders the collapsed `2 Steps` (or, once the turn has its own Say, the
// `2 Steps: {preview}` header) summary plus Say in ONE assistant bubble;
// while the turn still runs the message list shows NOTHING of its ladder —
// the pinned TURN LIVE REGION is its only surface. Opening
// the Turn reveals Steps; opening a Step reveals Thinking + N Function calls;
// opening that aggregate reveals call rows, and a call reveals its result.
// Collapse-all works both ways: Ctrl+L on window and the `⤒ 收起`
// link bump the epoch key on Bubble.List, remounting every bubble so all
// (uncontrolled) Collapses reset closed.
// ./sse.js is module-mocked like subagentBlock.dom.test.jsx (transitively
// imported via subagentBlock.jsx; no drill-in is opened here).

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';

const { openStreamMock } = vi.hoisted(() => ({ openStreamMock: vi.fn() }));
vi.mock('./sse.js', () => ({ openStream: openStreamMock }));
vi.mock('./api.js', () => ({
  apiGet: vi.fn().mockResolvedValue({}),
  apiPost: vi.fn().mockResolvedValue({}),
  apiDel: vi.fn().mockResolvedValue({}),
  authFetch: vi.fn(),
}));

import './test/setup-dom.js';
import { TranscriptView } from './transcript.jsx';
import { TurnLiveRegion } from './stepsBlock.jsx';

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

/// Live tests mount BOTH surfaces the chat page shows: the transcript AND
/// the TURN LIVE REGION pinned outside the message area (chat.jsx mounts it
/// between the scroll container and the input — a different thing from the
/// Say/Step bubbles).
const mountWithLiveRegion = (turns) => render(
  <>
    <TranscriptView turns={turns || [stepsTurn()]} usage={null} status="streaming" error={null} emptyText="无" />
    <TurnLiveRegion turns={turns || [stepsTurn()]} />
  </>,
);

/// Rerender BOTH surfaces together (same shape as mountWithLiveRegion).
const rerenderWithLiveRegion = (mounted, turns) => mounted.rerender(
  <>
    <TranscriptView turns={turns} usage={null} status="streaming" error={null} emptyText="无" />
    <TurnLiveRegion turns={turns} />
  </>,
);

afterEach(() => {
  cleanup();
});

describe('StepsContent three-level drill-down', () => {
  it('renders ONLY the group row at zero clicks — the whole ladder stays out of the DOM', () => {
    mount();
    expect(screen.getByText('2 Steps')).toBeTruthy();
    // Step, Thinking, function-call rows and results are all hidden.
    expect(screen.queryByText(/Step\(1\)/)).toBeNull();
    expect(screen.queryByText(/Step\(2\)/)).toBeNull();
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText(/Function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('ls -la')).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('clicking the group row reveals the step rows; thinking stays hidden (L0 → L1)', () => {
    mount();
    fireEvent.click(screen.getByText('2 Steps'));
    expect(screen.getByText(/Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/Step\(2\)/)).toBeTruthy();
    // Not drilled into Step(1) yet: no thinking, no aggregate row, no calls.
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/Function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
  });

  it('clicking a step row shows thinking + calls aggregate, not call rows', () => {
    mount();
    fireEvent.click(screen.getByText('2 Steps'));
    fireEvent.click(screen.getByText(/Step\(1\)/));
    expect(screen.getByText('💭 Thinking')).toBeTruthy();
    expect(screen.getByText('look at the repo first')).toBeTruthy();
    expect(screen.getByText('1 Function call')).toBeTruthy();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('ls -la')).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('clicking a single call row expands its exact input/output', () => {
    mount();
    fireEvent.click(screen.getByText('2 Steps'));
    fireEvent.click(screen.getByText(/Step\(1\)/));
    fireEvent.click(screen.getByText('1 Function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('ls -la')).toBeTruthy();
    expect(screen.getByText('total 8')).toBeTruthy();
  });

  it('keeps user disclosure state when new output rerenders the turn', () => {
    const initial = stepsTurn();
    // Settled turn: without `progressActive: false` the null-output
    // fallback marks the turn running and the live area would stream below
    // the ladder (it never opens anything, but keep the fixture settled).
    initial.progressActive = false;
    initial.steps[0].calls[0].output = null;
    const mounted = mount([initial]);
    fireEvent.click(screen.getByText('2 Steps'));
    fireEvent.click(screen.getByText(/Step\(1\)/));
    fireEvent.click(screen.getByText('1 Function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));

    const updated = stepsTurn();
    updated.progressActive = false;
    updated.steps[0].calls[0].output = 'new streamed output';
    mounted.rerender(
      <TranscriptView turns={[updated]} usage={null} status="streaming" error={null} emptyText="无" />,
    );

    expect(screen.getByText(/Step\(1\)/)).toBeTruthy();
    expect(screen.getByText('1 Function call')).toBeTruthy();
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText('new streamed output')).toBeTruthy();

    fireEvent.click(screen.getByText('2 Steps'));
    expect(
      screen.getByText('2 Steps').closest('.ant-collapse-item').classList.contains('ant-collapse-item-active'),
    ).toBe(false);
    updated.steps[0].calls[0].output = 'later output';
    mounted.rerender(
      <TranscriptView turns={[updated]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    expect(
      screen.getByText('2 Steps').closest('.ant-collapse-item').classList.contains('ant-collapse-item-active'),
    ).toBe(false);
  });

  it('Ctrl+L collapses the fully-drilled ladder back to the lone group row', () => {
    mount();
    fireEvent.click(screen.getByText('2 Steps'));
    fireEvent.click(screen.getByText(/Step\(1\)/));
    fireEvent.click(screen.getByText('1 Function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('total 8')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'l', ctrlKey: true });
    expect(screen.getByText('2 Steps')).toBeTruthy();
    expect(screen.queryByText(/Step\(1\)/)).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('the ⤒ 收起 link resets the ladder the same way', () => {
    mount();
    fireEvent.click(screen.getByText('2 Steps'));
    fireEvent.click(screen.getByText(/Step\(2\)/));
    fireEvent.click(screen.getByText('1 Function call'));
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
    fireEvent.click(screen.getByText('⤒ 收起'));
    expect(screen.getByText('2 Steps')).toBeTruthy();
    expect(screen.queryByText(/Step\(2\)/)).toBeNull();
    expect(screen.queryByText(/🔧 read/)).toBeNull();
  });

  it('a running turn (all calls finished, no Say yet) leaves the message list EMPTY', () => {
    const running = stepsTurn();
    running.progressActive = true;
    const { container } = mount([running]);
    // Every call landed but the turn is still pre-Say running: the L0 row
    // and the whole ladder stay OUT of the message list — no empty bubble
    // shell either (the pinned live region is the turn's only surface).
    expect(screen.queryByText('2 Steps')).toBeNull();
    expect(screen.queryByText('running')).toBeNull();
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(0);
  });

  it('a Say without sayActive retires the running tag and the header becomes the Say row', () => {
    // Even stale reducer state cannot show Running beside an existing Say:
    // once a Say streams in, the header switches to the Say form and the
    // running tag rides `sayActive` only (missing flag → no tag).
    const running = stepsTurn();
    running.progressActive = true;
    mount([running, { kind: 'text', role: 'assistant', text: 'Say started' }]);
    expect(screen.queryByText('running')).toBeNull();
    expect(screen.getByText('2 Steps')).toBeTruthy();
    expect(screen.queryByText(/Say\(/)).toBeNull();
  });

  it('keeps the running tag ON the Say row while sayActive — ladder FOLDED (12px gap)', () => {
    const streaming = stepsTurn();
    mount([
      { ...streaming, sayStreaming: true },
      { kind: 'text', role: 'assistant', text: 'partial answer line one\nmore below' },
    ]);
    const tag = screen.getByText('running');
    expect(tag).toBeTruthy();
    expect(tag.style.marginLeft).toBe('12px');
    expect(screen.getByText('2 Steps: partial answer line one')).toBeTruthy();
    expect(screen.queryByText('error')).toBeNull();
    // TUI parity: Say's first chunk settles the ladder — the running tag
    // rides the Say row while the steps stay folded under it.
    expect(screen.queryByText(/Step\(1\)/)).toBeNull();
    expect(screen.queryByText(/Function call/)).toBeNull();
  });

  it('shows the Say-row error tag once running settles and a call failed', () => {
    const errored = stepsTurn();
    errored.steps[0].calls[0].isError = true;
    mount([errored, { kind: 'text', role: 'assistant', text: 'finished with a failure' }]);
    expect(screen.getByText('2 Steps')).toBeTruthy();
    expect(screen.getByText('error')).toBeTruthy();
    expect(screen.queryByText('running')).toBeNull();
  });

  it('a hand-built turn with no progress flag and no open call stays folded (running is derived, not the stream status)', () => {
    const streaming = {
      kind: 'steps',
      role: 'assistant',
      steps: [{ thinking: 'planning the next call', calls: [] }],
    };
    mount([streaming]);
    expect(screen.getByText('1 Step')).toBeTruthy();
    expect(screen.queryByText(/Step\(1\)/)).toBeNull();
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText('planning the next call')).toBeNull();
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
    expect(screen.getByText('1 Step')).toBeTruthy();
    expect(screen.queryByText(/Step\(1\)/)).toBeNull();
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
    fireEvent.click(screen.getByText('2 Steps'));
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
    expect(screen.getByText('1 Step')).toBeTruthy();
    expect(screen.queryByText(/1 Steps/)).toBeNull();
    fireEvent.click(screen.getByText('1 Step'));
    fireEvent.click(screen.getByText(/Step\(1\)/));
    expect(screen.getByText('2 Function calls')).toBeTruthy();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText(/🔧 read/)).toBeNull();
    fireEvent.click(screen.getByText('2 Function calls'));
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
  });

  it('renders `N Steps + Say` as one visual Turn bubble', () => {
    const { container } = mount([
      stepsTurn(),
      { kind: 'text', role: 'assistant', text: 'all done here\nand the details follow' },
    ]);
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(1);
    // The completed Say keeps its full Markdown body below the step header.
    const group = screen.getByText('2 Steps');
    const say = container.querySelector('.md-body');
    expect(say.textContent).toContain('and the details follow');
    expect(group.compareDocumentPosition(say) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe('turn live region (its own area, outside the message list)', () => {
  // While a turn runs the message list shows NOTHING of its ladder — no L0
  // row, no steps content, no empty bubble shell: the TURN LIVE REGION is
  // its ONLY surface, a SEPARATE pinned area (never inside a bubble, never
  // inside the scroll list) gated only on running state, never on any
  // ladder's disclosure (TUI parity with chat_live_turn.rs). On settle the
  // folded ladder RETURNS inside the message list; it is user-controlled
  // and folded by default, so folded state is asserted through the item's
  // active class — never through text absence alone.
  const itemActive = (matcher) => screen.getByText(matcher)
    .closest('.ant-collapse-item').classList.contains('ant-collapse-item-active');
  const runningTurn = () => {
    const turn = stepsTurn();
    turn.progressActive = true;
    turn.steps[1].thinking = 'reading the entrypoint';
    return turn;
  };

  it('running turn streams ONLY in the live region — the message list shows nothing of its ladder', () => {
    const { container } = mountWithLiveRegion([runningTurn()]);
    // No L0 row, no ladder content, no leftover empty bubble shell.
    expect(screen.queryByText('2 Steps')).toBeNull();
    expect(screen.queryByText('running')).toBeNull();
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(0);
    // The LAST step's header + thinking + call row stream in the LIVE
    // REGION without a click.
    const region = screen.getByText('Step(2)').closest('[data-turn-live-region]');
    expect(region).toBeTruthy();
    expect(within(region).getByText('reading the entrypoint')).toBeTruthy();
    expect(within(region).getByText(/🔧 read/)).toBeTruthy();
    expect(within(region).queryByText(/Step\(1\)/)).toBeNull();
    expect(within(region).queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    // The region is NOT inside any transcript bubble.
    expect(region.closest('.ant-bubble')).toBeNull();
    // The region sits BELOW the (now ladder-less) message list in document
    // order.
    const list = container.querySelector('.ant-bubble-list');
    expect(list).toBeTruthy();
    expect(list.compareDocumentPosition(region) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('live region advances to the newest step; the message list still shows nothing', () => {
    const mounted = mountWithLiveRegion([runningTurn()]);
    expect(screen.getByText('Step(2)')).toBeTruthy();
    const grown = runningTurn();
    grown.steps.push({
      thinking: 'fresh reasoning',
      calls: [{
        kind: 'tool', role: 'assistant', id: 'c1', name: 'grep',
        input: 'pattern', output: null, isError: false, durationMs: 0, startedAt: 0,
      }],
    });
    rerenderWithLiveRegion(mounted, [grown]);
    // Still running: no `3 Steps` L0 row returns to the message list.
    expect(screen.queryByText('3 Steps')).toBeNull();
    const region = screen.getByText('Step(3)').closest('[data-turn-live-region]');
    expect(region).toBeTruthy();
    expect(within(region).getByText('fresh reasoning')).toBeTruthy();
    expect(within(region).getByText(/🔧 grep/)).toBeTruthy();
    // Only the LAST step streams: the previous one folded away.
    expect(within(region).queryByText('Step(2)')).toBeNull();
    expect(screen.queryByText('reading the entrypoint')).toBeNull();
  });

  it('streams the running step via the open-call fallback without a progress flag', () => {
    const fallback = stepsTurn();
    fallback.steps[1].thinking = 'reading the entrypoint';
    fallback.steps[1].calls[0].output = null;
    const { container } = mountWithLiveRegion([fallback]);
    // Derived running (open call, no progressActive field) drives BOTH
    // surfaces through the same predicate: message list empty, region on.
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(0);
    expect(screen.queryByText('2 Steps')).toBeNull();
    const region = screen.getByText('Step(2)').closest('[data-turn-live-region]');
    expect(within(region).getByText('reading the entrypoint')).toBeTruthy();
    expect(within(region).getByText(/🔧 read/)).toBeTruthy();
  });

  it('settle rerender returns the folded ladder to the message list and clears the region', () => {
    const mounted = mountWithLiveRegion([runningTurn()]);
    expect(screen.queryByText('2 Steps')).toBeNull();
    expect(screen.getByText('reading the entrypoint')).toBeTruthy();
    // The Say's first chunk settles the turn: rerender in Say-preview label
    // mode (sayStreaming) — the folded ladder is BACK inside the message
    // list as the `N Steps: {preview}` Say row.
    rerenderWithLiveRegion(mounted, [
      { ...stepsTurn(), sayStreaming: true },
      { kind: 'text', role: 'assistant', text: 'all wrapped up' },
    ]);
    const header = screen.getByText('2 Steps: all wrapped up');
    expect(header.closest('.ant-bubble')).toBeTruthy();
    expect(itemActive('2 Steps: all wrapped up')).toBe(false);
    expect(screen.queryByText(/Step\(2\)/)).toBeNull();
    expect(screen.queryByText('reading the entrypoint')).toBeNull();
    expect(screen.queryByText(/🔧 read/)).toBeNull();
    expect(screen.queryByText(/live/)).toBeNull();
    expect(document.querySelector('[data-turn-live-region]')).toBeNull();
  });

  it('after settle the ladder opens/closes on pure user control and the state survives rerenders', () => {
    const mounted = mountWithLiveRegion([runningTurn()]);
    // While running there is no L0 row in the message list to click — the
    // live region is the only surface.
    expect(screen.queryByText('2 Steps')).toBeNull();
    rerenderWithLiveRegion(mounted, [stepsTurn()]);
    // Fresh mount of the settled ladder: folded, step rows not even mounted.
    expect(screen.queryByText(/Step\(2\)/)).toBeNull();
    fireEvent.click(screen.getByText('2 Steps'));
    expect(itemActive('2 Steps')).toBe(true);
    expect(screen.getByText(/Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/Step\(2\)/)).toBeTruthy();
    // Uncontrolled L0: a rerender of the settled turn leaves the user's
    // open state alone, and the live region stays gone (no coupling).
    const updated = stepsTurn();
    updated.steps[0].calls[0].output = 'later output';
    rerenderWithLiveRegion(mounted, [updated]);
    expect(itemActive('2 Steps')).toBe(true);
    expect(document.querySelector('[data-turn-live-region]')).toBeNull();
    // Closing is plain user control too (antd keeps once-opened panels
    // mounted, so folded state reads from the item's active class).
    fireEvent.click(screen.getByText('2 Steps'));
    expect(itemActive('2 Steps')).toBe(false);
  });

  it('running tag appears on live region call rows, not on finished ones', () => {
    const running = runningTurn();
    running.steps[1].calls.push({
      kind: 'tool', role: 'assistant', id: 'c1', name: 'grep',
      input: 'pattern', output: null, isError: false, durationMs: 0, startedAt: 0,
    });
    mountWithLiveRegion([running]);
    // No L0 row exists in the message list while the turn runs, so the ONLY
    // running tag is the open call row inside the live region.
    expect(screen.getAllByText('running')).toHaveLength(1);
    const grepRow = screen.getByText(/🔧 grep/).closest('.ant-collapse-item');
    expect(within(grepRow).getByText('running')).toBeTruthy();
    const readRow = screen.getByText(/🔧 read/).closest('.ant-collapse-item');
    expect(readRow.textContent).not.toContain('running');
  });

  it('flat `tool` turns share the same per-call running tag while open', () => {
    mount([{
      kind: 'tool', role: 'assistant', id: 't1', name: 'task',
      input: '', output: null, isError: false, durationMs: 0, startedAt: 0,
    }]);
    expect(screen.getByText(/🔧 task/)).toBeTruthy();
    expect(screen.getByText('running')).toBeTruthy();
  });
});

describe('completed Say Markdown and streaming preview', () => {
  it('replaces the raw streaming preview with one semantic Markdown document on completion', () => {
    const say = { kind: 'text', role: 'assistant', text: '# Stream result\n\n**finished**' };
    const mounted = mount([{ ...stepsTurn(), sayStreaming: true }, say]);
    expect(screen.getByText('2 Steps: # Stream result')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'Stream result' })).toBeNull();
    mounted.rerender(<TranscriptView turns={[stepsTurn(), say]} status="done" />);
    expect(screen.getByRole('heading', { name: 'Stream result' })).toBeTruthy();
    expect(screen.getAllByText('Stream result')).toHaveLength(1);
    expect(screen.getByText('finished').tagName).toBe('STRONG');
    expect(screen.queryByText('running')).toBeNull();
  });

  it('renders the complete Markdown document once with spacing below the ladder', () => {
    const { container } = mount([
      stepsTurn(),
      { kind: 'text', role: 'assistant', text: '# Result\n\n**Done**\n\n- first\n- second\n\n```js\nconst ok = true;\n```\n\n| key | value |\n| --- | --- |\n| status | good |\n\n[Open](https://example.com)' },
    ]);
    expect(screen.getByText('2 Steps')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Result', level: 1 })).toBeTruthy();
    expect(screen.getAllByText('Result')).toHaveLength(1);
    expect(screen.getByText('Done').tagName).toBe('STRONG');
    expect(container.querySelectorAll('.md-body li')).toHaveLength(2);
    expect(container.querySelector('pre code').textContent).toContain('const ok = true;');
    expect(container.querySelector('table td').textContent).toBe('status');
    expect(screen.getByRole('link', { name: 'Open' }).getAttribute('href')).toBe('https://example.com');
    expect(container.querySelector('.md-body').parentElement.style.marginTop).toBe('16px');
  });

  it('renders a completed one-line Say as Markdown without duplicating its text in the header', () => {
    const { container } = mount([stepsTurn(), { kind: 'text', role: 'assistant', text: '**all done here**' }]);
    expect(screen.getByText('2 Steps')).toBeTruthy();
    expect(screen.getByText('all done here').tagName).toBe('STRONG');
    expect(container.textContent.split('all done here')).toHaveLength(2);
  });
});
