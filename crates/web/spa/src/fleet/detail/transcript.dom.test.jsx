// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ExecutionTranscript } from '../detail.jsx';
import { turnsFromMessages } from '../../reduce.js';
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), authFetch: vi.fn() }));
vi.mock('../../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

const messages = [
  { role: 'user', blocks: [{ kind: 'text', text: 'inspect repo' }] },
  { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'check path' }, { kind: 'tool_use', id: 'a', name: 'bash', input: { command: 'pwd' } }] },
  { role: 'tool', blocks: [{ kind: 'tool_result', tool_use_id: 'a', content: '/repo', is_error: false }] },
  { role: 'assistant', blocks: [{ kind: 'text', text: '# complete\n**details**' }] },
];

describe('execution detail shares the TUI Say/Step ladder', () => {
  it('pairs separate persisted messages and retains nested disclosure on refresh', () => {
    const view = render(<ExecutionTranscript messages={messages} />);
    expect(screen.getByText('Say(1 step)')).toBeTruthy();
    expect(screen.getByText('details').tagName).toBe('STRONG');
    expect(screen.queryByText('—')).toBeNull();
    expect(screen.queryByText('/repo')).toBeNull();
    fireEvent.click(screen.getByText('Say(1 step)'));
    fireEvent.click(screen.getByText('Step(1)'));
    expect(screen.getByText('check path')).toBeTruthy();
    fireEvent.click(screen.getByText('1 Function call'));
    fireEvent.click(screen.getByText('🔧 bash'));
    expect(screen.getByText('/repo')).toBeTruthy();
    view.rerender(<ExecutionTranscript messages={structuredClone(messages)} />);
    expect(screen.getByText('/repo')).toBeTruthy();
    fireEvent.click(screen.getByText('⤒ 收起'));
    expect(screen.queryByText('/repo')).toBeNull();
    expect(screen.getByText('details')).toBeTruthy();
  });
  it('does not turn presentation-only image blocks into a Say heading', () => {
    render(<ExecutionTranscript messages={[
      messages[1], messages[2], { role: 'assistant', blocks: [{ kind: 'image', url: 'data:image/png;base64,AA==' }] },
    ]} />);
    expect(screen.getByText('1 Step')).toBeTruthy();
    expect(screen.queryByText(/Say\(/)).toBeNull();
    expect(screen.getByText('[image]')).toBeTruthy();
  });
  it('retains an expanded ladder when live status rows disappear on snapshot settlement', () => {
    const turns = turnsFromMessages(messages);
    const view = render(<ExecutionTranscript messages={messages} live={{ turns: [
      turns[0], { kind: 'sys', text: 'agent → act' }, ...turns.slice(1),
    ] }} />);
    fireEvent.click(screen.getByText('Say(1 step)'));
    fireEvent.click(screen.getByText('Step(1)'));
    expect(screen.getByText('check path')).toBeTruthy();
    view.rerender(<ExecutionTranscript messages={structuredClone(messages)} />);
    expect(screen.getByText('check path')).toBeTruthy();
    expect(screen.queryByText('agent → act')).toBeNull();
  });
});
