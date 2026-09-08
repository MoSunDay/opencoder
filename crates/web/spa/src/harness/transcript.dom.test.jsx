// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { ExecutionTranscript } from '../fleet/detail.jsx';
import { emptyStream, reduceFrame } from '../reduce.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), authFetch: vi.fn() }));
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));
afterEach(cleanup);

const messages = [
  { role: 'user', blocks: [{ kind: 'text', text: 'inspect files' }] },
  { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'Inspect the resource' }] },
  { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'turn:item', name: 'bash', input: { command: 'cat file' } }] },
  { role: 'tool', blocks: [{ kind: 'tool_result', tool_use_id: 'turn:item', content: 'FILE_RESULT', is_error: false }] },
  { role: 'assistant', blocks: [{ kind: 'text', text: 'Task complete' }] },
];
function drill() {
  fireEvent.click(screen.getByText('1 Step'));
  fireEvent.click(screen.getByText('Step(1)'));
  expect(screen.getByText('Inspect the resource')).toBeTruthy();
  fireEvent.click(screen.getByText('1 Function call'));
  fireEvent.click(screen.getByText('🔧 bash'));
  expect(screen.getByText('FILE_RESULT')).toBeTruthy();
}
it('renders normalized Codex items in the existing collapsed ladder', () => {
  render(<ExecutionTranscript messages={messages} />);
  expect(screen.getByText('Task complete')).toBeTruthy();
  expect(screen.queryByText('FILE_RESULT')).toBeNull();
  drill();
});
it('uses the same ladder for live normalized events and a refreshed snapshot', () => {
  let live = emptyStream();
  for (const [event, data] of [
    ['steer_consumed', { seq: 1, text: 'inspect files' }],
    ['llm_round_start', { started_at_ms: 100 }],
    ['reasoning_delta', { text: 'Inspect the resource' }],
    ['tool_start', { id: 'turn:item', name: 'bash', input: { command: 'cat file' } }],
    ['tool_end', { id: 'turn:item', name: 'bash', output: 'FILE_RESULT', is_error: false }],
    ['text_delta', { text: 'Task complete' }],
    ['llm_round_end', {}], ['done', {}],
  ]) { live = reduceFrame(live, { event, data }, 150); }
  const view = render(<ExecutionTranscript messages={[]} live={live} />);
  expect(screen.getByText('Task complete')).toBeTruthy();
  drill();
  view.rerender(<ExecutionTranscript messages={messages} />);
  expect(screen.getByText('Task complete')).toBeTruthy();
  expect(screen.getByText('FILE_RESULT')).toBeTruthy();
});
