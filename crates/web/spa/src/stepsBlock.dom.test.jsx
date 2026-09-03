// @vitest-environment jsdom
// StepsBlock DOM drill-down: the THREE-LEVEL ladder (group row → step rows →
// thinking + calls-aggregate → per-call rows). Zero clicks render ONLY the
// L0 group row (`❯ 2 steps [running|error]`); each level needs one click to
// reveal the next. Say stays a separate top-level ai bubble AFTER the steps
// bubble. Collapse-all works both ways: Ctrl+L on window and the `⤒ 收起`
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
    expect(screen.getByText('❯ 2 steps')).toBeTruthy();
    // L1 step rows, L2 thinking + aggregate row, L3 call rows: all hidden.
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText(/❯ Step\(2\)/)).toBeNull();
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText(/function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('ls -la')).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('clicking the group row reveals the step rows; thinking stays hidden (L0 → L1)', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    expect(screen.getByText(/❯ Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/❯ Step\(2\)/)).toBeTruthy();
    // Not drilled into Step(1) yet: no thinking, no aggregate row, no calls.
    expect(screen.queryByText('💭 Thinking')).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
  });

  it('clicking a step row shows thinking DIRECTLY + the calls-aggregate row (L1 → L2)', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    // Thinking is a direct render inside the step — no ghost collapse level.
    expect(screen.getByText('💭 Thinking')).toBeTruthy();
    expect(screen.getByText('look at the repo first')).toBeTruthy();
    // The step's calls collapsed behind the aggregate row.
    expect(screen.getByText('❯ 1 function call')).toBeTruthy();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
  });

  it('clicking the aggregate row reveals call rows but not their payloads (L2 → L3)', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    fireEvent.click(screen.getByText('❯ 1 function call'));
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.queryByText('ls -la')).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('clicking a single call row expands its exact input/output', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    fireEvent.click(screen.getByText('❯ 1 function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('ls -la')).toBeTruthy();
    expect(screen.getByText('total 8')).toBeTruthy();
  });

  it('Ctrl+L collapses the fully-drilled ladder back to the lone group row', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    fireEvent.click(screen.getByText('❯ 1 function call'));
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('total 8')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'l', ctrlKey: true });
    expect(screen.getByText('❯ 2 steps')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(1\)/)).toBeNull();
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/function call/)).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('the ⤒ 收起 link resets the ladder the same way', () => {
    mount();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    fireEvent.click(screen.getByText(/❯ Step\(2\)/));
    expect(screen.getByText('❯ 1 function call')).toBeTruthy();
    fireEvent.click(screen.getByText('⤒ 收起'));
    expect(screen.getByText('❯ 2 steps')).toBeTruthy();
    expect(screen.queryByText(/❯ Step\(2\)/)).toBeNull();
    expect(screen.queryByText(/function call/)).toBeNull();
  });

  it('shows the running tag on the group row while a call is open', () => {
    const running = stepsTurn();
    running.steps[0].calls[0].output = null;
    mount([running]);
    expect(screen.getByText('running')).toBeTruthy();
    expect(screen.queryByText('error')).toBeNull();
  });

  it('shows the error tag on the group row (and on the failed step once expanded)', () => {
    const errored = stepsTurn();
    errored.steps[0].calls[0].isError = true;
    mount([errored]);
    // Zero clicks: only the group-row tag is in the document.
    expect(screen.getByText('error')).toBeTruthy();
    expect(screen.queryByText('running')).toBeNull();
    fireEvent.click(screen.getByText('❯ 2 steps'));
    // Step(1) carries the failed call → its own red tag next to the label.
    expect(screen.getAllByText('error').length).toBe(2);
  });

  it('singular/plural labels: 1 step → `❯ 1 step`; 2 calls → `❯ 2 function calls`', () => {
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
    expect(screen.getByText('❯ 1 step')).toBeTruthy();
    expect(screen.queryByText(/❯ 1 steps/)).toBeNull();
    fireEvent.click(screen.getByText('❯ 1 step'));
    fireEvent.click(screen.getByText(/❯ Step\(1\)/));
    expect(screen.getByText('❯ 2 function calls')).toBeTruthy();
  });

  it('renders Say as a separate top-level bubble AFTER the steps bubble', () => {
    const { container } = mount([stepsTurn(), { kind: 'text', role: 'assistant', text: 'all done here' }]);
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(2);
    const group = screen.getByText('❯ 2 steps');
    const say = screen.getByText('all done here');
    expect(group.compareDocumentPosition(say) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
