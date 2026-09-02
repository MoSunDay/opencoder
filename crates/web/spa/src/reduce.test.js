// vitest smoke tests for the pure transcript reducers (reduce.js).
import { describe, expect, it } from 'vitest';
import { consumedEchoText, deltaTextOf, emptyStream, nestedEventOf, reduceFrame, turnsFromMessages, withUserTurn } from './reduce.js';

describe('turnsFromMessages', () => {
  it('flattens text/tool blocks and attaches tool_result to the open tool', () => {
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
    expect(wire[1]).toMatchObject({ kind: 'think', role: 'assistant', text: 'thinking' });
    expect(wire[2]).toMatchObject({ kind: 'tool', name: 'bash', output: 'a.txt', isError: false });
    expect(turns[2]).toMatchObject({ kind: 'tool', name: 'bash', output: 'a.txt', isError: false });
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

  it('pairs tool_start/tool_end and derives duration from arrival times', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'x', name: 'bash', input: { cmd: 'ls' } } }, 1000);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'x', name: 'bash', output: 'ok', is_error: false } }, 2500);
    expect(s.turns[0]).toMatchObject({ kind: 'tool', name: 'bash', output: 'ok', durationMs: 1500 });
  });

  it('prefers an explicit duration_ms on the end frame', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'x', name: 't', input: {} } }, 1000);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'x', output: 'o', duration_ms: 42 } }, 9999);
    expect(s.turns[0].durationMs).toBe(42);
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
    expect(st.turns[0].events[1]).toMatchObject({ kind: 'tool', name: 'bash' });
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
