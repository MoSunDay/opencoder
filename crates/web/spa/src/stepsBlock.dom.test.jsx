// @vitest-environment jsdom
// StepsBlock DOM smoke: a `steps` turn renders the static `≡ N steps`
// marker and BOTH per-step rows immediately (NO group collapse — the ladder
// is always visible), with thinking/call content hidden until a step header
// is clicked; call rows then expand to their input/output paragraphs.
// Collapse-all is exercised both ways: Ctrl+L on window and the `⤒ 收起`
// link bump the epoch key on Bubble.List, remounting every bubble closed.
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

describe('StepsContent ladder', () => {
  it('renders the marker and both step rows immediately, content hidden', () => {
    mount();
    expect(screen.getByText(/≡ 2 steps/)).toBeTruthy();
    expect(screen.getByText(/❯ Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/❯ Step\(2\)/)).toBeTruthy();
    // Default closed: neither the step thinking nor the call row is rendered.
    expect(screen.queryByText('look at the repo first')).toBeNull();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
  });

  it('shows running while a call is open, error once a finished call failed', () => {
    const running = stepsTurn();
    running.steps[0].calls[0].output = null;
    mount([running]);
    expect(screen.getByText('running')).toBeTruthy();
    expect(screen.queryByText('error')).toBeNull();

    cleanup();
    const errored = stepsTurn();
    errored.steps[0].calls[0].isError = true;
    mount([errored]);
    // Marker-level error (nothing running) + the errored step's own tag.
    expect(screen.getAllByText('error').length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText('running')).toBeNull();
  });

  it('expands a step to its thinking row + call row, then the call to input/output', () => {
    const { container } = mount();
    const headers = container.querySelectorAll('.ant-collapse-header');
    fireEvent.click(headers[0]); // ❯ Step(1)
    expect(screen.getByText('💭 Thinking')).toBeTruthy();
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    // The think row is itself a collapsed collapse: its text stays hidden.
    expect(screen.queryByText('look at the repo first')).toBeNull();
    // Open the call row → exact fixture strings for input/output.
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('ls -la')).toBeTruthy();
    expect(screen.getByText('total 8')).toBeTruthy();
  });

  it('Ctrl+L collapses everything expanded; step rows remain', () => {
    const { container } = mount();
    fireEvent.click(container.querySelectorAll('.ant-collapse-header')[0]);
    fireEvent.click(screen.getByText(/🔧 bash/));
    expect(screen.getByText('total 8')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'l', ctrlKey: true });
    expect(screen.getByText(/≡ 2 steps/)).toBeTruthy();
    expect(screen.getByText(/❯ Step\(1\)/)).toBeTruthy();
    expect(screen.getByText(/❯ Step\(2\)/)).toBeTruthy();
    expect(screen.queryByText(/🔧 bash/)).toBeNull();
    expect(screen.queryByText('total 8')).toBeNull();
  });

  it('the ⤒ 收起 link resets the ladder the same way', () => {
    const { container } = mount();
    fireEvent.click(container.querySelectorAll('.ant-collapse-header')[1]); // Step(2)
    expect(screen.getByText(/🔧 read/)).toBeTruthy();
    fireEvent.click(screen.getByText('⤒ 收起'));
    expect(screen.getByText(/❯ Step\(2\)/)).toBeTruthy();
    expect(screen.queryByText(/🔧 read/)).toBeNull();
  });

  it('renders Say as a separate top-level bubble AFTER the steps bubble', () => {
    const { container } = mount([stepsTurn(), { kind: 'text', role: 'assistant', text: 'all done here' }]);
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(2);
    const marker = screen.getByText(/≡ 2 steps/);
    const say = screen.getByText('all done here');
    expect(marker.compareDocumentPosition(say) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
