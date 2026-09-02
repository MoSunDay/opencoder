// @vitest-environment jsdom
// SubagentBlock DOM smoke: a done subagent turn renders its fold header
// (🤖 name · status + tag + closed-state summary), expanding reveals the
// folded child events, and [→ view] opens the child-session replay modal —
// openStream folds frames via reduceFrame into the same compact list, and
// abort() runs when the modal closes. ./sse.js is module-mocked (same
// pattern as sse.test.js); ./api.js is mocked because sse.js consumes it.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { openStreamMock, abortMock } = vi.hoisted(() => ({
  openStreamMock: vi.fn(),
  abortMock: vi.fn(),
}));
vi.mock('./sse.js', () => ({ openStream: openStreamMock }));
vi.mock('./api.js', () => ({
  apiGet: vi.fn().mockResolvedValue({}),
  apiPost: vi.fn().mockResolvedValue({}),
  apiDel: vi.fn().mockResolvedValue({}),
  signFetch: vi.fn(),
}));

import './test/setup-dom.js';
import { TranscriptView } from './transcript.jsx';
import { childLines, statusColorOf, usageTextOf } from './subagentBlock.jsx';

const turn = () => ({
  kind: 'subagent',
  id: 'sa1',
  name: 'explore',
  prompt: 'look around',
  childSessionId: 'child-9',
  status: 'done',
  ok: true,
  summary: 'found 3 files',
  events: [
    { kind: 'text', role: 'assistant', text: 'child says hi' },
    { kind: 'think', role: 'assistant', text: 'child thinking' },
    { kind: 'tool', role: 'assistant', name: 'bash', input: 'ls', output: 'a.txt', isError: true, durationMs: 5 },
    { kind: 'sys', text: 'status: running' },
  ],
  usage: { input: 3, output: 4, total: 7, contextWindow: null },
  startedAt: 0,
});

const mount = () => render(
  <TranscriptView turns={[turn()]} usage={null} status="done" error={null} emptyText="无" />,
);

beforeEach(() => {
  openStreamMock.mockReset().mockReturnValue({ abort: abortMock });
  abortMock.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('childLines / helpers (pure)', () => {
  it('maps child event kinds and appends the usage line', () => {
    const lines = childLines(turn());
    expect(lines.map((l) => l.kind)).toEqual(['text', 'think', 'tool', 'sys', 'usage']);
    expect(lines[2]).toMatchObject({ kind: 'tool', text: 'bash', isError: true });
    expect(lines[4].text).toBe('Σ 7 tokens');
    expect(childLines(null)).toEqual([]);
    expect(childLines({ events: 'nope' })).toEqual([]);
    expect(childLines({ events: [], usage: null })).toEqual([]);
  });

  it('maps statuses to tag colors and formats usage', () => {
    expect(statusColorOf('running')).toBe('processing');
    expect(statusColorOf('done')).toBe('success');
    expect(statusColorOf('error')).toBe('error');
    expect(statusColorOf('cancelled')).toBe('default');
    expect(usageTextOf(null)).toBeNull();
    expect(usageTextOf({ total: 12 })).toBe('Σ 12 tokens');
  });
});

describe('SubagentContent in the transcript', () => {
  it('renders the fold header with status tag and closed-state summary', () => {
    mount();
    expect(screen.getByText(/🤖 explore · done/)).toBeTruthy();
    expect(screen.getByText('found 3 files')).toBeTruthy(); // summary while closed
    // Collapsed by default — child events stay hidden until expanded.
    expect(screen.queryByText('child says hi')).toBeNull();
  });

  it('expands to reveal the folded child events', async () => {
    const { container } = mount();
    fireEvent.click(container.querySelector('.ant-collapse-header'));
    await waitFor(() => {
      expect(screen.getByText('child says hi')).toBeTruthy();
    });
    expect(screen.getByText('child thinking')).toBeTruthy();
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText('status: running')).toBeTruthy();
    expect(screen.getByText('Σ 7 tokens')).toBeTruthy();
  });

  it('drills into the child session replay and aborts on close', async () => {
    mount();
    fireEvent.click(screen.getByText('[→ view]'));
    await waitFor(() => {
      expect(openStreamMock).toHaveBeenCalledTimes(1);
    });
    const cfg = openStreamMock.mock.calls[0][0];
    expect(cfg.path).toBe('/api/sessions/child-9/events');
    expect(cfg.sessionId).toBe('child-9');
    expect(cfg.after).toBe(0);
    // The replay modal carries the compact list, not a Bubble.List.
    expect(await screen.findByText(/子会话回放/)).toBeTruthy();
    expect(document.querySelector('.ant-modal .ant-bubble-list')).toBeNull();
    expect(screen.getByText('暂无子会话事件')).toBeTruthy();
    // Folding a live child frame through reduceFrame updates the replay.
    await act(async () => {
      cfg.onFrame({ event: 'text_delta', data: { text: 'replayed line' } });
    });
    expect(screen.getByText('replayed line')).toBeTruthy();
    // Closing the modal aborts the replay stream.
    fireEvent.click(document.querySelector('.ant-modal-close'));
    await waitFor(() => {
      expect(abortMock).toHaveBeenCalledTimes(1);
    });
  });

  it('hides the drill-in link when the child session id is missing', () => {
    render(
      <TranscriptView
        turns={[{ ...turn(), childSessionId: null }]}
        usage={null}
        status="done"
        error={null}
        emptyText="无"
      />,
    );
    expect(screen.getByText(/🤖 explore · done/)).toBeTruthy();
    expect(screen.queryByText('[→ view]')).toBeNull();
  });
});
