// vitest smoke tests for the pure transcript reducers (reduce.js).
import { describe, expect, it } from 'vitest';
import { consumedEchoText, deltaTextOf, emptyStream, nestedEventOf, reduceFrame, turnsFromMessages, withUserTurn } from './reduce.js';

describe('turnsFromMessages', () => {
  it('flattens text/tool blocks and attaches tool_result to the open call', () => {
    const turns = turnsFromMessages([
      { role: 'user', blocks: [{ type: 'text', text: 'hi' }] },
      {
        role: 'assistant',
        blocks: [
          { type: 'text', text: 'working' },
          { type: 'tool_use', id: 't1', name: 'bash', input: { cmd: 'ls' } },
          { type: 'tool_result', tool_use_id: 't1', output: 'a.txt', is_error: false },
        ],
      },
    ]);
    expect(turns).toHaveLength(3);
    expect(turns[0]).toMatchObject({ kind: 'text', role: 'user', text: 'hi' });
    expect(turns[1]).toMatchObject({ kind: 'text', role: 'assistant', text: 'working' });
    // One assistant message's non-task tool_uses = ONE step inside a steps
    // turn; the same-message tool_result backfills the buffered call by id.
    expect(turns[2]).toMatchObject({ kind: 'steps', role: 'assistant' });
    expect(turns[2].steps).toHaveLength(1);
    expect(turns[2].steps[0].calls[0]).toMatchObject({ kind: 'tool', name: 'bash', output: 'a.txt', isError: false });
    // serde wire tag is `kind` (crates/core/src/message.rs) — the real
    // contract; a `type`-only matcher returned [] and blanked every store
    // replay (caught by real-browser acceptance).
    const wire = turnsFromMessages([
      { role: 'user', blocks: [{ kind: 'text', text: 'wire-hi' }] },
      {
        role: 'assistant',
        blocks: [
          { kind: 'reasoning', text: 'thinking' },
          { kind: 'tool_use', id: 'w1', name: 'bash', input: { cmd: 'ls' } },
          { kind: 'tool_result', tool_use_id: 'w1', content: 'a.txt', is_error: false },
        ],
      },
    ]);
    expect(wire[0]).toMatchObject({ kind: 'text', role: 'user', text: 'wire-hi' });
    // The trailing reasoning run is absorbed into the step's thinking
    // (coalesce_steps) instead of staying a standalone think turn.
    expect(wire[1]).toMatchObject({
      kind: 'steps',
      steps: [{ thinking: 'thinking', calls: [{ kind: 'tool', name: 'bash', output: 'a.txt', isError: false }] }],
    });
  });

  it('tolerates missing messages/blocks', () => {
    expect(turnsFromMessages(undefined)).toEqual([]);
    expect(turnsFromMessages([{ role: 'user', blocks: undefined }])).toEqual([]);
  });

  // Echo contract: the recorded user turn keeps the verbatim input (with the
  // `$skill` token) in `display` while `blocks` carry the resolved clean text
  // the LLM consumes. The snapshot transcript must render `display`.
  it('renders user display text verbatim over recorded blocks', () => {
    const turns = turnsFromMessages([
      {
        role: 'user',
        display: '$review fix the bug',
        blocks: [{ kind: 'text', text: ' fix the bug' }],
      },
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0]).toMatchObject({ kind: 'text', role: 'user', text: '$review fix the bug' });
  });

  it('skips synthetic user messages unless they carry display text', () => {
    // Internal markers (handoff/compaction): skipped, like the TUI replay.
    const internal = turnsFromMessages([
      { role: 'user', synthetic: true, blocks: [{ kind: 'text', text: 'internal' }] },
    ]);
    expect(internal).toEqual([]);
    // Skill trigger standing in for a pure `$review` submit: display IS the
    // user's own words — rendered.
    const trigger = turnsFromMessages([
      {
        role: 'user',
        synthetic: true,
        display: '$review',
        blocks: [{ kind: 'text', text: 'The active skill is now in effect.' }],
      },
    ]);
    expect(trigger).toHaveLength(1);
    expect(trigger[0]).toMatchObject({ kind: 'text', role: 'user', text: '$review' });
  });

  it('falls back to blocks for legacy rows without display', () => {
    const turns = turnsFromMessages([
      { role: 'user', blocks: [{ kind: 'text', text: 'legacy prompt' }] },
    ]);
    expect(turns[0]).toMatchObject({ kind: 'text', role: 'user', text: 'legacy prompt' });
  });
});

describe('reduceFrame', () => {
  it('accumulates assistant text deltas, accepting text and delta fields', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'he' } }, 0);
    s = reduceFrame(s, { event: 'text_delta', data: { delta: 'llo' } }, 1);
    expect(s.turns).toEqual([{ kind: 'text', role: 'assistant', text: 'hello', open: true }]);
  });

  it('starts a new text turn after a closed one', () => {
    let s = reduceFrame(emptyStream(), { event: 'text_delta', data: { text: 'a' } }, 0);
    s = reduceFrame(s, { event: 'done', data: {} }, 1);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'b' } }, 2);
    expect(s.turns.map((t) => t.text)).toEqual(['a', 'b']);
  });

  it('pairs tool_start/tool_end into a step and derives duration from arrival times', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'x', name: 'bash', input: { cmd: 'ls' } } }, 1000);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'x', name: 'bash', output: 'ok', is_error: false } }, 2500);
    expect(s.turns[0]).toMatchObject({ kind: 'steps', role: 'assistant' });
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ kind: 'tool', name: 'bash', output: 'ok', durationMs: 1500 });
  });

  it('prefers an explicit duration_ms on the end frame', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'x', name: 't', input: {} } }, 1000);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'x', output: 'o', duration_ms: 42 } }, 9999);
    expect(s.turns[0].steps[0].calls[0].durationMs).toBe(42);
  });

  it('records llm_usage and terminal error state', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'llm_usage', data: { input_tokens: 10, output_tokens: 5, total_tokens: 15 } }, 0);
    s = reduceFrame(s, { event: 'error', data: { error: 'boom' } }, 1);
    expect(s.usage).toMatchObject({ input: 10, output: 5, total: 15 });
    expect(s.status).toBe('error');
    expect(s.error).toBe('boom');
  });

  it('treats a lag-marked error as a re-sync signal, not a terminal failure', () => {
    // api.rs map_broadcast_result synthesizes {error, lag} when a consumer
    // falls behind; sse.js reconnects and the run usually keeps producing.
    // reduceFrame must not latch terminal 'error' for it (chat.jsx releases
    // busy on terminal errors — a lag frame would free a running turn).
    let s = reduceFrame(emptyStream(), { event: 'text_delta', data: { text: 'partial' } }, 0);
    s = reduceFrame(s, { event: 'error', data: { error: 'event lag: 5 events dropped', lag: 5 } }, 1);
    expect(s.status).not.toBe('error');
    expect(s.error).toBe(null);
    // A real error frame (no lag marker) still terminates.
    s = reduceFrame(s, { event: 'error', data: { error: 'boom' } }, 2);
    expect(s.status).toBe('error');
    expect(s.error).toBe('boom');
  });

  it('done closes the stream state and open text', () => {
    let s = reduceFrame(emptyStream(), { event: 'text_delta', data: { text: 'x' } }, 0);
    s = reduceFrame(s, { event: 'done', data: {} }, 1);
    expect(s.status).toBe('done');
    expect(s.turns[0].open).toBe(false);
  });

  it('echoes consumed queue/steer prompts as user turns', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'queue_consumed', data: { text: 'do it' } }, 0);
    expect(s.turns[0]).toMatchObject({ kind: 'text', role: 'user', text: 'do it' });
    // Compound control command: the server normalizes the echo to the tail
    // (the only part entering context); the SPA renders it verbatim.
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: 'review the code' } }, 1);
    expect(s.turns[1]).toMatchObject({ kind: 'text', role: 'user', text: 'review the code' });
    // Bare control command: applied inline, nothing recorded — no echo turn.
    const bare = reduceFrame(s, { event: 'queue_consumed', data: { text: '' } }, 2);
    expect(bare.turns.length).toBe(s.turns.length);
  });

  // Agent surfaces render the wire value verbatim: the sandbox->plan rename
  // (legacy "sandbox" is gone, resolve_agent("sandbox") is None server-side)
  // must show "plan" as-is, and any unknown future value must degrade to an
  // empty name rather than crash the stream.
  it('renders agent_switched sys turns for plan and unknown values', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'agent_switched', data: { agent: 'plan' } }, 0);
    expect(s.turns[0]).toEqual({ kind: 'sys', text: 'agent → plan' });
    s = reduceFrame(s, { event: 'agent_switched', data: { agent: 'act' } }, 1);
    expect(s.turns[1]).toEqual({ kind: 'sys', text: 'agent → act' });
    s = reduceFrame(s, { event: 'agent_switched', data: {} }, 2);
    expect(s.turns[2]).toEqual({ kind: 'sys', text: 'agent → ' });
  });

  it('ignores unknown events without mutating state', () => {
    const s = emptyStream();
    expect(reduceFrame(s, { event: 'subagent_child', data: {} }, 0)).toBe(s);
  });
});

describe('deltaTextOf/withUserTurn', () => {
  it('prefers text over delta and handles non-strings', () => {
    expect(deltaTextOf({ text: 'a', delta: 'b' })).toBe('a');
    expect(deltaTextOf({ delta: 'b' })).toBe('b');
    expect(deltaTextOf({})).toBe('');
    expect(deltaTextOf({ text: 3 })).toBe('');
  });

  it('withUserTurn appends a user text turn', () => {
    const s = withUserTurn(emptyStream(), 'hi');
    expect(s.turns).toEqual([{ kind: 'text', role: 'user', text: 'hi' }]);
    expect(withUserTurn(emptyStream(), '')).toEqual(emptyStream());
  });

  // Mirror of control_cmd.rs::consumed_echo_tails_compound_suppresses_bare_keeps_plain.
  it('consumedEchoText tails compound suppresses bare keeps plain', () => {
    expect(consumedEchoText('review the code')).toBe('review the code');
    expect(consumedEchoText('/plan review')).toBe('review');
    expect(consumedEchoText('/act_clear_context finish the summary')).toBe('finish the summary');
    expect(consumedEchoText('/plan')).toBe('');
    expect(consumedEchoText('/act   ')).toBe('');
    expect(consumedEchoText('/clear_context')).toBe('');
    // Unknown slash words are plain prompts — echoed verbatim.
    expect(consumedEchoText('/foo bar')).toBe('/foo bar');
  });

  it('remote optimistic echo never renders a bare control command', () => {
    expect(withUserTurn(emptyStream(), consumedEchoText('/plan'))).toEqual(emptyStream());
    const compound = withUserTurn(emptyStream(), consumedEchoText('/plan review the diff'));
    expect(compound.turns).toEqual([{ kind: 'text', role: 'user', text: 'review the diff' }]);
  });
});

import { usageFromMessages } from './reduce.js';

describe('usageFromMessages (store snapshot → footer)', () => {
  it('sums per-message usage from the wire shape', () => {
    const u = usageFromMessages([
      { role: 'user', blocks: [], usage: { input_tokens: 100, output_tokens: 0, total_tokens: 100 } },
      { role: 'assistant', blocks: [], usage: { input_tokens: 0, output_tokens: 342, total_tokens: 342 } },
      { role: 'tool', blocks: [], usage: { input_tokens: 5, output_tokens: 5, total_tokens: 10 } },
    ]);
    expect(u).toEqual({ input: 105, output: 347, total: 452, contextWindow: null });
  });

  it('returns null when no row carries usage (no empty footer)', () => {
    expect(usageFromMessages([{ role: 'user', blocks: [] }])).toBeNull();
    expect(usageFromMessages([])).toBeNull();
    expect(usageFromMessages([{ role: 'user', blocks: [], usage: {} }])).toBeNull();
  });
});

describe('subagent folding', () => {
  const start = { event: 'subagent_start', data: { id: 'sa1', kind: 'explore', prompt: 'look around', child_session_id: 'child-9' } };

  it('opens a foldable subagent block from subagent_start', () => {
    let st = reduceFrame(emptyStream(), start, 0);
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0]).toMatchObject({
      kind: 'subagent', id: 'sa1', name: 'explore', prompt: 'look around',
      childSessionId: 'child-9', status: 'running', ok: null,
    });
  });

  it('folds nested externally-tagged child events into the block', () => {
    let st = reduceFrame(emptyStream(), start, 0);
    st = reduceFrame(st, { event: 'subagent_child', data: { id: 'sa1', event: { TextDelta: 'hello' } } }, 1);
    st = reduceFrame(st, { event: 'subagent_child', data: { id: 'sa1', event: { ToolStart: { id: 't1', name: 'bash', input: { cmd: 'ls' } } } } }, 2);
    expect(st.turns[0].events).toHaveLength(2);
    expect(st.turns[0].events[0]).toMatchObject({ kind: 'text', text: 'hello' });
    // Child tool calls fold into the SAME steps ladder as the main stream.
    expect(st.turns[0].events[1]).toMatchObject({
      kind: 'steps',
      steps: [{ calls: [{ kind: 'tool', id: 't1', name: 'bash', output: null }] }],
    });
  });

  it('closes the block from subagent_end with ok/summary/cancelled', () => {
    let st = reduceFrame(emptyStream(), start, 0);
    st = reduceFrame(st, { event: 'subagent_end', data: { id: 'sa1', ok: true, cancelled: false, summary: 'found it' } }, 3);
    expect(st.turns[0]).toMatchObject({ status: 'done', ok: true, summary: 'found it' });
    st = reduceFrame(emptyStream(), start, 0);
    st = reduceFrame(st, { event: 'subagent_end', data: { id: 'sa1', ok: false, cancelled: true, summary: '' } }, 3);
    expect(st.turns[0]).toMatchObject({ status: 'cancelled', ok: false });
  });

  it('ignores child frames for unknown ids', () => {
    const st = reduceFrame(emptyStream(), { event: 'subagent_child', data: { id: 'nope', event: { Done: {} } } }, 0);
    expect(st.turns).toHaveLength(0);
  });
});

describe('steps ladder (live, mirror of chat_steps.rs)', () => {
  const startCall = (id, name, at) => reduceFrame(
    emptyStream(),
    { event: 'tool_start', data: { id, name, input: { cmd: id } } },
    at,
  );

  it('(a) same-round parallel calls merge into ONE step and both backfill', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1000);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 1100);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'out-a', is_error: false } }, 2000);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'b', name: 'read', output: 'out-b', is_error: true } }, 2100);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].steps).toHaveLength(1);
    const calls = s.turns[0].steps[0].calls;
    expect(calls).toHaveLength(2);
    expect(calls[0]).toMatchObject({ id: 'a', name: 'bash', output: 'out-a', isError: false, durationMs: 1000 });
    expect(calls[1]).toMatchObject({ id: 'b', name: 'read', output: 'out-b', isError: true, durationMs: 1000 });
  });

  it('(b) sequential rounds (A ends before B starts) open TWO steps in ONE group', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'x', is_error: false } }, 2);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 3);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].kind).toBe('steps');
    expect(s.turns[0].steps).toHaveLength(2);
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ id: 'a', output: 'x' });
    expect(s.turns[0].steps[1].calls[0]).toMatchObject({ id: 'b', output: null });
  });

  it("(c) absorbs the segment's think runs — trailing OR before a Say", () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'plan it' } }, 1);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 2);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].steps[0].thinking).toBe('plan it');
    // A tool turn never leaves free-floating thinking above the ladder: the
    // Say stays a top-level bubble and the thinking folds into the step.
    let t = emptyStream();
    t = reduceFrame(t, { event: 'reasoning_delta', data: { text: 'before say' } }, 1);
    t = reduceFrame(t, { event: 'text_delta', data: { text: 'Say!' } }, 2);
    t = reduceFrame(t, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 3);
    expect(t.turns.map((x) => x.kind)).toEqual(['text', 'steps']);
    expect(t.turns[0]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Say!' });
    expect(t.turns[1].steps[0].thinking).toBe('before say');
  });

  it("(c2) think runs on both sides of a Say concatenate earliest-first into one step", () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'first ' } }, 1);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'mid Say' } }, 2);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'second' } }, 3);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 4);
    expect(s.turns.map((x) => x.kind)).toEqual(['text', 'steps']);
    expect(s.turns[0]).toMatchObject({ kind: 'text', role: 'assistant', text: 'mid Say' });
    expect(s.turns[1].steps[0].thinking).toBe('first second');
  });

  it('(c3) user and sys boundaries fold the pre-boundary think run into a call-less step', () => {
    // Steer/queue echo is a user turn: a hard segment boundary. The
    // pre-boundary think run folds right there — segment attribution is
    // preserved (it stays before the echo), just inside the ladder.
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'old segment' } }, 1);
    s = reduceFrame(s, { event: 'queue_consumed', data: { text: 'steered' } }, 2);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 3);
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text', 'steps']);
    expect(s.turns[0].steps[0]).toMatchObject({ thinking: 'old segment', calls: [] });
    expect(s.turns[1]).toMatchObject({ kind: 'text', role: 'user', text: 'steered' });
    expect(s.turns[2].steps[0].thinking).toBe('');
    // sys markers (status/compaction) are boundaries too.
    let t = emptyStream();
    t = reduceFrame(t, { event: 'reasoning_delta', data: { text: 'pre-marker' } }, 1);
    t = reduceFrame(t, { event: 'status', data: { status: 'thinking' } }, 2);
    t = reduceFrame(t, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 3);
    expect(t.turns.map((x) => x.kind)).toEqual(['steps', 'sys', 'steps']);
    expect(t.turns[0].steps[0]).toMatchObject({ thinking: 'pre-marker', calls: [] });
    expect(t.turns[2].steps[0].thinking).toBe('');
  });

  it('(c4) a pure-text round (no tool call ever) folds its think run into a call-less step at done', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'ponder' } }, 1);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'final answer' } }, 2);
    s = reduceFrame(s, { event: 'done', data: {} }, 3);
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text']);
    expect(s.turns[0].steps[0]).toMatchObject({ thinking: 'ponder', calls: [] });
    expect(s.turns[1]).toMatchObject({ kind: 'text', text: 'final answer' });
  });

  it('(d) Say between two rounds opens a NEW steps turn (Say stays top-level)', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'x', is_error: false } }, 2);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'interlude answer' } }, 3);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 4);
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text', 'steps']);
    expect(s.turns[1]).toMatchObject({ kind: 'text', role: 'assistant', text: 'interlude answer' });
  });

  it('(e) orphan tool_end synthesizes a finished call folded into the ladder', () => {
    const s = reduceFrame(emptyStream(), { event: 'tool_end', data: { id: 'ghost', name: 'bash', output: 'late', is_error: true } }, 5);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].kind).toBe('steps');
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({
      id: 'ghost', name: 'bash', input: null, output: 'late', isError: true, durationMs: 0,
    });
  });

  it('(f) the task tool keeps its flat tool turn (subagent block renders it)', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'task1', name: 'task', input: { prompt: 'x' } } }, 1);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0]).toMatchObject({ kind: 'tool', name: 'task', output: null });
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'task1', name: 'task', output: 'child done', is_error: false } }, 2);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0]).toMatchObject({ kind: 'tool', name: 'task', output: 'child done' });
  });

  it('tool_update follows tool_start folding (same shape, open call)', () => {
    const s = startCall('u1', 'bash', 7);
    expect(s.turns[0]).toMatchObject({ kind: 'steps' });
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ id: 'u1', name: 'bash', output: null, startedAt: 7 });
  });
});

describe('steps ladder (snapshot, message-pair semantics)', () => {
  it('(g) reasoning + 2 tool_use + results + Say → text, steps(backfilled), text', () => {
    const turns = turnsFromMessages([
      { role: 'user', blocks: [{ kind: 'text', text: 'go' }] },
      {
        role: 'assistant',
        blocks: [
          { kind: 'reasoning', text: 'must run tools' },
          { kind: 'tool_use', id: 'a', name: 'bash', input: { cmd: 'ls' } },
          { kind: 'tool_use', id: 'b', name: 'read', input: { path: 'x' } },
        ],
      },
      {
        role: 'tool',
        blocks: [
          { kind: 'tool_result', tool_use_id: 'a', output: 'ls-out', is_error: false },
          { kind: 'tool_result', tool_use_id: 'b', output: 'read-err', is_error: true },
        ],
      },
      { role: 'assistant', blocks: [{ kind: 'text', text: 'final answer' }] },
    ]);
    expect(turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    expect(turns[0]).toMatchObject({ role: 'user', text: 'go' });
    expect(turns[1].steps).toHaveLength(1);
    expect(turns[1].steps[0].thinking).toBe('must run tools');
    expect(turns[1].steps[0].calls.map((c) => c.name)).toEqual(['bash', 'read']);
    expect(turns[1].steps[0].calls[0]).toMatchObject({ output: 'ls-out', isError: false });
    expect(turns[1].steps[0].calls[1]).toMatchObject({ output: 'read-err', isError: true });
    // Say is a TOP-LEVEL text turn after the group, never folded in.
    expect(turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'final answer' });
  });

  it('(h) two adjacent assistant tool messages fold into ONE steps turn, 2 steps', () => {
    const turns = turnsFromMessages([
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'a', name: 'bash', input: {} }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'b', name: 'read', input: {} }] },
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0].kind).toBe('steps');
    expect(turns[0].steps).toHaveLength(2);
    expect(turns[0].steps.map((st) => st.calls[0].id)).toEqual(['a', 'b']);
  });

  it('(i) a task tool_use stays a flat tool turn and takes its result', () => {
    const turns = turnsFromMessages([
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 't1', name: 'task', input: { prompt: 'x' } }] },
      { role: 'user', blocks: [{ kind: 'tool_result', tool_use_id: 't1', output: 'child summary', is_error: false }] },
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0]).toMatchObject({ kind: 'tool', name: 'task', output: 'child summary', isError: false });
  });

  it('(j) reasoning-only message + later tool message folds the think into that step', () => {
    const turns = turnsFromMessages([
      { role: 'user', blocks: [{ kind: 'text', text: 'go' }] },
      { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'plan' }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'a', name: 'bash', input: {} }] },
      { role: 'user', synthetic: true, blocks: [{ kind: 'tool_result', tool_use_id: 'a', output: 'ok', is_error: false }] },
    ]);
    // No free-floating think turn above the ladder — message-pair semantics.
    expect(turns.map((t) => t.kind)).toEqual(['text', 'steps']);
    expect(turns[1].steps).toHaveLength(1);
    expect(turns[1].steps[0].thinking).toBe('plan');
  });

  it('(k) reasoning before a mid-run Say folds into the FOLLOWING round (live parity)', () => {
    // Say mid-run, tools after it: the reasoning joins the later step and
    // the Say stays a top-level bubble — same fold as the live
    // absorbSegmentThinking (c).
    const turns = turnsFromMessages([
      { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'hmm' }, { kind: 'text', text: 'let me check' }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'a', name: 'bash', input: {} }] },
    ]);
    expect(turns.map((t) => t.kind)).toEqual(['text', 'steps']);
    expect(turns[0]).toMatchObject({ kind: 'text', role: 'assistant', text: 'let me check' });
    expect(turns[1].steps[0].thinking).toBe('hmm');
  });

  it('(l) reasoning + text with NO tool after folds the think run into a call-less step', () => {
    const turns = turnsFromMessages([
      { role: 'user', blocks: [{ kind: 'text', text: 'q' }] },
      { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'final thought' }, { kind: 'text', text: 'done' }] },
    ]);
    expect(turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    expect(turns[1].steps[0]).toMatchObject({ thinking: 'final thought', calls: [] });
  });

  it('(m) a user boundary stops the fold — reasoning belongs to the PREVIOUS segment', () => {
    const turns = turnsFromMessages([
      { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'a1' }, { kind: 'text', text: 'say one' }] },
      { role: 'user', blocks: [{ kind: 'text', text: 'q2' }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'z', name: 'bash', input: {} }] },
    ]);
    expect(turns.map((t) => t.kind)).toEqual(['steps', 'text', 'text', 'steps']);
    // The reasoning stays BEFORE the user boundary (previous segment) — now
    // inside a call-less step instead of a bare think turn.
    expect(turns[0].steps[0]).toMatchObject({ thinking: 'a1', calls: [] });
    expect(turns[3].steps[0].thinking).toBe('');
  });

  it('folds trailing pending reasoning into a call-less step at the end of the walk', () => {
    const turns = turnsFromMessages([
      { role: 'assistant', blocks: [{ kind: 'reasoning', text: 'dangling thought' }] },
    ]);
    expect(turns).toEqual([
      { kind: 'steps', role: 'assistant', steps: [{ thinking: 'dangling thought', calls: [] }] },
    ]);
  });
});

describe('nestedEventOf', () => {
  it('converts serde externally-tagged variants to SSE form', () => {
    expect(nestedEventOf({ TextDelta: 'abc' })).toEqual({ event: 'text_delta', data: { text: 'abc' } });
    expect(nestedEventOf({ LlmUsage: { total_tokens: 5 } })).toEqual({ event: 'llm_usage', data: { total_tokens: 5 } });
    expect(nestedEventOf({ Status: 'retry 1/3' })).toEqual({ event: 'status', data: { status: 'retry 1/3' } });
    expect(nestedEventOf({})).toBe(null);
    expect(nestedEventOf('nope')).toBe(null);
  });
});

describe('status-line frames', () => {
  it('compaction_delta renders a sys line', () => {
    const st = reduceFrame(emptyStream(), { event: 'compaction_delta', data: { text: 'folding 40 turns' } }, 0);
    expect(st.turns[0]).toMatchObject({ kind: 'sys', text: '🗜 folding 40 turns' });
  });

  it('autopilot renders phase + iteration', () => {
    const st = reduceFrame(emptyStream(), { event: 'autopilot', data: { phase: 'Plan', iteration: 2 } }, 0);
    expect(st.turns[0]).toMatchObject({ kind: 'sys', text: '🛸 autopilot plan #2' });
  });

  it('interrupted renders a sys line', () => {
    const st = reduceFrame(emptyStream(), { event: 'interrupted', data: {} }, 0);
    expect(st.turns[0]).toMatchObject({ kind: 'sys', text: '⏹ interrupted' });
  });

  it('transcript_reset closes open text (snapshot reload is chat.jsx\'s job)', () => {
    let st = emptyStream();
    st = reduceFrame(st, { event: 'text_delta', data: { text: 'partial' } }, 0);
    st = reduceFrame(st, { event: 'transcript_reset', data: {} }, 1);
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0].open).toBeFalsy();
  });
});
