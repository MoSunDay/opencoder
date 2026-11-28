// @vitest-environment jsdom
// Subagent fold DOM tests — the transcript-wide disclosure (TUI
// toggle_subagents parity, subagentFold.js): every subagent block header
// carries the ladder's shared ❯ / ▸ glyph (disclosureIcon, stepsBlock.jsx),
// the bodies stay hidden by default, and clicking ANY subagent header opens
// or closes ALL blocks of the transcript at once. Ctrl+L and the `⤒ 收起`
// link close them too (transcript.jsx collapseAll), while the [→ view]
// drill-in link leaves the disclosure alone. Mounted outside a transcript
// (no provider) a block falls back to its own toggle. ./sse.js is
// module-mocked and ./api.js is mocked because sse.js consumes it (same
// pattern as subagentBlock.dom.test.jsx).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';

const { openStreamMock, abortMock } = vi.hoisted(() => ({
  openStreamMock: vi.fn(),
  abortMock: vi.fn(),
}));
vi.mock('./sse.js', () => ({ openStream: openStreamMock }));
vi.mock('./api.js', () => ({
  apiGet: vi.fn().mockResolvedValue({}),
  apiPost: vi.fn().mockResolvedValue({}),
  apiDel: vi.fn().mockResolvedValue({}),
  authFetch: vi.fn(),
}));

import './test/setup-dom.js';
import { TranscriptView } from './transcript.jsx';
import { SubagentContent } from './subagentBlock.jsx';

const FIRST_TEXT = 'first child says hi';
const SECOND_TEXT = 'second child says hi';

const subagentTurn = (id, name, text) => ({
  kind: 'subagent',
  id,
  name,
  prompt: 'look around',
  childSessionId: 'child-' + id,
  status: 'done',
  ok: true,
  summary: 'found 3 files in ' + id,
  events: [
    { kind: 'text', role: 'assistant', text },
    { kind: 'tool', role: 'assistant', name: 'ls', input: '.', output: 'a.txt', isError: false, durationMs: 1 },
  ],
  usage: { input: 3, output: 4, total: 7, contextWindow: null },
  startedAt: 0,
});

const firstTurn = () => subagentTurn('sa1', 'explore', FIRST_TEXT);
const secondTurn = () => subagentTurn('sa2', 'review', SECOND_TEXT);

const mount = () => render(
  <TranscriptView
    turns={[firstTurn(), secondTurn()]}
    usage={null}
    status="done"
    error={null}
    emptyText="无"
  />,
);

/// Every collapse header of the mounted transcript, in DOM order (two
/// subagent blocks in these fixtures; nothing else collapses).
const headers = () => Array.from(document.querySelectorAll('.ant-collapse-header'));

/// The disclosure glyph of one header: the expandIcon span's own text.
const glyphOf = (header) => within(header).getByText(/^[▸❯]$/).textContent;
const glyphs = () => headers().map(glyphOf);

/// Open state per header: antd toggles `ant-collapse-item-active` on the
/// item. A once-opened panel KEEPS its DOM when closed again, so the class —
/// not the absence of the text — is the closed signal.
const itemsActive = () => headers().map((h) => (
  h.closest('.ant-collapse-item').classList.contains('ant-collapse-item-active')
));

beforeEach(() => {
  openStreamMock.mockReset().mockReturnValue({ abort: abortMock });
  abortMock.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('transcript-wide subagent disclosure', () => {
  it('renders the ▸ marker on every subagent block and hides child lines by default', () => {
    mount();
    expect(headers()).toHaveLength(2);
    expect(glyphs()).toEqual(['▸', '▸']);
    // The glyphs are the only ▸ in the transcript (no antd default arrow).
    expect(screen.getAllByText('▸')).toHaveLength(2);
    expect(screen.queryByText('❯')).toBeNull();
    expect(itemsActive()).toEqual([false, false]);
    expect(screen.queryByText(FIRST_TEXT)).toBeNull();
    expect(screen.queryByText(SECOND_TEXT)).toBeNull();
    // Closed-state summaries stand in for the hidden bodies.
    expect(screen.getByText('found 3 files in sa1')).toBeTruthy();
    expect(screen.getByText('found 3 files in sa2')).toBeTruthy();
  });

  it('clicking one subagent header toggles every subagent block open, then closed', () => {
    mount();
    fireEvent.click(headers()[0]);
    expect(glyphs()).toEqual(['❯', '❯']);
    expect(itemsActive()).toEqual([true, true]);
    expect(screen.getByText(FIRST_TEXT)).toBeTruthy();
    expect(screen.getByText(SECOND_TEXT)).toBeTruthy();
    // Open blocks drop the closed-state summary.
    expect(screen.queryByText('found 3 files in sa1')).toBeNull();
    expect(screen.queryByText('found 3 files in sa2')).toBeNull();
    // Clicking the OTHER header closes them all again — one state for all.
    fireEvent.click(headers()[1]);
    expect(glyphs()).toEqual(['▸', '▸']);
    expect(itemsActive()).toEqual([false, false]);
    expect(screen.getByText('found 3 files in sa1')).toBeTruthy();
    expect(screen.getByText('found 3 files in sa2')).toBeTruthy();
  });

  it('Ctrl+L collapses every subagent block', () => {
    mount();
    fireEvent.click(headers()[0]);
    expect(itemsActive()).toEqual([true, true]);
    fireEvent.keyDown(window, { key: 'l', ctrlKey: true });
    expect(glyphs()).toEqual(['▸', '▸']);
    expect(itemsActive()).toEqual([false, false]);
    // The epoch remount also drops the once-rendered bodies from the DOM.
    expect(screen.queryByText(FIRST_TEXT)).toBeNull();
    expect(screen.queryByText(SECOND_TEXT)).toBeNull();
  });

  it('⤒ 收起 collapses every subagent block', () => {
    mount();
    fireEvent.click(headers()[1]);
    expect(itemsActive()).toEqual([true, true]);
    fireEvent.click(screen.getByText('⤒ 收起'));
    expect(glyphs()).toEqual(['▸', '▸']);
    expect(itemsActive()).toEqual([false, false]);
    expect(screen.queryByText(FIRST_TEXT)).toBeNull();
    expect(screen.queryByText(SECOND_TEXT)).toBeNull();
  });

  it('the [→ view] drill-in link does not toggle the disclosure', () => {
    mount();
    fireEvent.click(screen.getAllByText('[→ view]')[0]);
    // The drill-in itself still opens the child replay.
    expect(openStreamMock).toHaveBeenCalledTimes(1);
    expect(openStreamMock.mock.calls[0][0].path).toBe('/api/sessions/child-sa1/events');
    expect(screen.getByText(/子会话回放/)).toBeTruthy();
    // …while every block stays closed.
    expect(glyphs()).toEqual(['▸', '▸']);
    expect(itemsActive()).toEqual([false, false]);
    expect(screen.queryByText(FIRST_TEXT)).toBeNull();
  });
});

describe('useSubagentFold without a provider', () => {
  it('falls back to a block-local toggle outside TranscriptView', () => {
    render(
      <>
        <SubagentContent turn={firstTurn()} />
        <SubagentContent turn={secondTurn()} />
      </>,
    );
    expect(glyphs()).toEqual(['▸', '▸']);
    fireEvent.click(headers()[0]);
    // Only the clicked block opens — there is no transcript to share with.
    expect(glyphs()).toEqual(['❯', '▸']);
    expect(itemsActive()).toEqual([true, false]);
    expect(screen.getByText(FIRST_TEXT)).toBeTruthy();
    expect(screen.queryByText(SECOND_TEXT)).toBeNull();
    fireEvent.click(headers()[0]);
    expect(glyphs()).toEqual(['▸', '▸']);
    expect(itemsActive()).toEqual([false, false]);
  });
});
