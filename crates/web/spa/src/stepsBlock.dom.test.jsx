// @vitest-environment jsdom
// StepsBlock DOM drill-down: Turn → Step → Function call. Zero clicks render
// the collapsed `2 Steps` (or, once the turn has its own Say, the
// `2 Steps: {preview}` header) summary plus Say in ONE assistant
// bubble. Opening
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
    // fallback marks the turn running and the live auto-open would open
    // levels without any click.
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

describe('live ladder disclosure (auto-open + user override)', () => {
  // antd keeps closed-after-open panels mounted (hidden), so folded state is
  // asserted through the item's active class, exactly like the user-state
  // test above — never through text absence.
  const itemActive = (matcher) => screen.getByText(matcher)
    .closest('.ant-collapse-item').classList.contains('ant-collapse-item-active');

  it('auto-opens the running position while progressActive — older steps stay folded', () => {
    const running = stepsTurn();
    running.progressActive = true;
    mount([running]);
    // L0 + the LAST step + its calls aggregate are open WITHOUT clicks.
    expect(itemActive('2 Steps')).toBe(true);
    expect(screen.getByText(/Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/Step\(2\)/)).toBeTruthy();
    expect(itemActive(/Step\(2\)/)).toBe(true);
    expect(itemActive('1 Function call')).toBe(true);
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
    // The older step keeps its fold: only its ROW is visible.
    expect(itemActive(/Step\(1\)/)).toBe(false);
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
  });

  it('auto-opens the same running position via the open-call fallback (output: null)', () => {
    const running = stepsTurn();
    running.steps[1].calls[0].output = null;
    mount([running]);
    expect(itemActive('2 Steps')).toBe(true);
    expect(itemActive(/Step\(2\)/)).toBe(true);
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
    expect(itemActive(/Step\(1\)/)).toBe(false);
    expect(screen.queryByText('look at the repo first')).toBeNull();
  });

  it('auto-collapses everything back to the `2 Steps` summary when the turn settles', () => {
    const running = stepsTurn();
    running.progressActive = true;
    const mounted = mount([running]);
    expect(itemActive('2 Steps')).toBe(true);
    mounted.rerender(
      <TranscriptView turns={[stepsTurn()]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    expect(itemActive('2 Steps')).toBe(false);
    expect(itemActive(/Step\(2\)/)).toBe(false);
    expect(itemActive('1 Function call')).toBe(false);
  });

  it('an expanded call row pins its step and calls open across the settle', () => {
    const running = stepsTurn();
    running.progressActive = true;
    const mounted = mount([running]);
    // The auto-opened running position puts the LAST step's call row in
    // reach; drill into that call result (no explicit ladder clicks).
    fireEvent.click(screen.getByText(/🔧 read/));
    expect(screen.getByText('fn main() {}')).toBeTruthy();
    mounted.rerender(
      <TranscriptView turns={[stepsTurn()]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    // The call row pinned its enclosing step + calls aggregate open, so the
    // drilled result stays visible (user override beats the emptied auto).
    expect(itemActive(/Step\(2\)/)).toBe(true);
    expect(itemActive('1 Function call')).toBe(true);
    expect(screen.getByText('fn main() {}')).toBeTruthy();
    // Other levels collapse as usual: the turn row and the older step.
    expect(itemActive('2 Steps')).toBe(false);
    expect(itemActive(/Step\(1\)/)).toBe(false);
  });

  it('a step born while running moves the auto slot: the old step folds by itself', () => {
    const running = stepsTurn();
    running.progressActive = true;
    const mounted = mount([running]);
    expect(itemActive(/Step\(2\)/)).toBe(true);
    const grown = stepsTurn();
    grown.progressActive = true;
    grown.steps.push({ thinking: 'fresh reasoning', calls: [] });
    mounted.rerender(
      <TranscriptView turns={[grown]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    expect(screen.getByText('3 Steps')).toBeTruthy();
    expect(itemActive(/Step\(2\)/)).toBe(false);
    expect(screen.getByText(/Step\(3\)/)).toBeTruthy();
    expect(itemActive(/Step\(3\)/)).toBe(true);
    expect(screen.getByText('fresh reasoning')).toBeTruthy();
  });

  it('a user-closed level wins over the auto state across streaming rerenders', () => {
    const running = stepsTurn();
    running.progressActive = true;
    const mounted = mount([running]);
    expect(itemActive('2 Steps')).toBe(true);
    fireEvent.click(screen.getByText('2 Steps'));
    expect(itemActive('2 Steps')).toBe(false);
    const still = stepsTurn();
    still.progressActive = true;
    mounted.rerender(
      <TranscriptView turns={[still]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    // The auto keys say open, but the user override keeps the turn folded.
    expect(itemActive('2 Steps')).toBe(false);
    expect(itemActive(/Step\(2\)/)).toBe(true);
  });

  it('a user-opened OLD step stays open when the auto slot moves on', () => {
    const running = stepsTurn();
    running.progressActive = true;
    const mounted = mount([running]);
    fireEvent.click(screen.getByText(/Step\(1\)/));
    expect(screen.getByText('look at the repo first')).toBeTruthy();
    const grown = stepsTurn();
    grown.progressActive = true;
    grown.steps.push({ thinking: 'fresh reasoning', calls: [] });
    mounted.rerender(
      <TranscriptView turns={[grown]} usage={null} status="streaming" error={null} emptyText="无" />,
    );
    expect(itemActive(/Step\(1\)/)).toBe(true);
    expect(screen.getByText('look at the repo first')).toBeTruthy();
    expect(itemActive(/Step\(3\)/)).toBe(true);
  });

  it('shows the running tag on a running call row, not on finished ones', () => {
    const running = stepsTurn();
    running.steps[1].calls.push({
      kind: 'tool', role: 'assistant', id: 'c1', name: 'grep',
      input: 'pattern', output: null, isError: false, durationMs: 0, startedAt: 0,
    });
    mount([running]);
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
