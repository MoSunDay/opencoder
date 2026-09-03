// vitest smoke tests for the pure transcript reducers (reduce.js).
import { describe, expect, it } from 'vitest';
import { consumedEchoText, deltaTextOf, emptyStream, ensurePendingEcho, nestedEventOf, reduceFrame, resyncState, turnsFromMessages, withUserTurn } from './reduce.js';

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
    // The leading text block CLOSES the round: it lands above, and the
    // message's non-task tool_uses form ONE step inside a NEW steps turn
    // below it (floor contract); the same-message tool_result backfills the
    // buffered call by id.
    expect(turns[1]).toMatchObject({ kind: 'text', role: 'assistant', text: 'working' });
    expect(turns[2]).toMatchObject({ kind: 'steps', role: 'assistant' });
    expect(turns[2].progressActive).toBe(false);
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

  // Pairing contract (TUI replay block-order): one user input owns one or
  // MORE pairs of (n Steps + Say) — each non-empty assistant text block
  // closes its sub-turn, so the tool round AFTER a Say opens a fresh ladder
  // below it instead of merging into the group above. Tool results arrive
  // as `role: 'tool'` carrier messages (core Role enum; synthetic user rows
  // are skipped entirely).
  it('pairs Say-closed sub-turns: [user, steps[a], say mid, steps[b], say final]', () => {
    const turns = turnsFromMessages([
      { role: 'user', blocks: [{ kind: 'text', text: 'go' }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'a', name: 'bash', input: {} }] },
      { role: 'tool', blocks: [{ kind: 'tool_result', tool_use_id: 'a', output: 'A', is_error: false }] },
      { role: 'assistant', blocks: [{ kind: 'text', text: 'mid say' }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'b', name: 'read', input: {} }] },
      { role: 'tool', blocks: [{ kind: 'tool_result', tool_use_id: 'b', output: 'B', is_error: false }] },
      { role: 'assistant', blocks: [{ kind: 'text', text: 'final say' }] },
    ]);
    expect(turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text', 'steps', 'text']);
    expect(turns[0]).toMatchObject({ role: 'user', text: 'go' });
    expect(turns[1].steps[0].calls.map((c) => c.id)).toEqual(['a']);
    expect(turns[1].steps[0].calls[0]).toMatchObject({ output: 'A', isError: false });
    expect(turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'mid say' });
    expect(turns[3].steps[0].calls.map((c) => c.id)).toEqual(['b']);
    expect(turns[3].steps[0].calls[0]).toMatchObject({ output: 'B', isError: false });
    expect(turns[4]).toMatchObject({ kind: 'text', role: 'assistant', text: 'final say' });
  });

  it('skips empty assistant text blocks (TUI replay parity): neither render nor close', () => {
    const turns = turnsFromMessages([
      {
        role: 'assistant',
        blocks: [
          { kind: 'tool_use', id: 'a', name: 'bash', input: {} },
          { kind: 'text', text: '   ' },
          { kind: 'text', text: 'done' },
        ],
      },
    ]);
    expect(turns.map((t) => t.kind)).toEqual(['steps', 'text']);
    expect(turns[0].steps[0].calls.map((c) => c.id)).toEqual(['a']);
    expect(turns[1]).toMatchObject({ role: 'assistant', text: 'done' });
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

  it('keeps step progress after ToolEnd and settles it on the first Say chunk', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'x', name: 'bash', input: {} } }, 1);
    expect(s.turns[0].progressActive).toBe(true);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'x', name: 'bash', output: 'ok' } }, 2);
    expect(s.turns[0].progressActive).toBe(true);
    const unchanged = reduceFrame(s, { event: 'text_delta', data: { text: '' } }, 3);
    expect(unchanged).toBe(s);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Say starts' } }, 4);
    expect(s.turns[0].progressActive).toBe(false);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'another phase' } }, 5);
    expect(s.turns[0].progressActive).toBe(false);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'y', name: 'bash', input: {} } }, 6);
    expect(s.turns[0].progressActive).toBe(false);
  });

  it('settles step progress at terminal boundaries when no Say arrives', () => {
    let done = reduceFrame(emptyStream(), { event: 'reasoning_delta', data: { text: 'thinking' } }, 1);
    expect(done.turns[0].progressActive).toBe(true);
    done = reduceFrame(done, { event: 'done', data: {} }, 2);
    expect(done.turns[0].progressActive).toBe(false);

    let failed = reduceFrame(emptyStream(), { event: 'tool_start', data: { id: 'x', name: 'bash', input: {} } }, 1);
    failed = reduceFrame(failed, { event: 'error', data: { error: 'boom' } }, 2);
    expect(failed.turns[0].progressActive).toBe(false);
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
    // The child folds through the same reduceFrame: its Say is a floor, so
    // the tool call joins a NEW steps ladder BELOW the text.
    expect(st.turns[0].events[0]).toMatchObject({ kind: 'text', text: 'hello' });
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

// TUI pending_turn_echo parity (chat_types.rs / replay.rs rebuild_after_reset):
// the in-flight turn's user echo is remembered from submit/steer/queue echo
// until done/error, survives transcript_reset, and a store-snapshot rebuild
// re-pushes it when the snapshot lacks it (ensurePendingEcho).
describe('pendingEcho (TUI pending_turn_echo parity)', () => {
  it('steer/queue consumed echoes are remembered as pendingEcho', () => {
    let s = reduceFrame(emptyStream(), { event: 'queue_consumed', data: { text: 'tail summary' } }, 0);
    expect(s.pendingEcho).toBe('tail summary');
    // A later steer in the same run replaces the remembered echo.
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: 'second steer' } }, 1);
    expect(s.pendingEcho).toBe('second steer');
  });

  it('transcript_reset preserves pendingEcho; done/error clear it', () => {
    let s = reduceFrame(emptyStream(), { event: 'steer_consumed', data: { text: 'tail' } }, 0);
    s = reduceFrame(s, { event: 'transcript_reset', data: {} }, 1);
    // The reset rebuilds from a snapshot that has NOT recorded the echo yet.
    expect(s.pendingEcho).toBe('tail');
    s = reduceFrame(s, { event: 'done', data: {} }, 2);
    expect(s.pendingEcho).toBe(null);

    let e = reduceFrame(emptyStream(), { event: 'queue_consumed', data: { text: 'boom tail' } }, 0);
    e = reduceFrame(e, { event: 'error', data: { error: 'x' } }, 1);
    expect(e.pendingEcho).toBe(null);
    // A lag-marked error is a consumer re-sync, not a terminal — echo survives.
    let lag = reduceFrame(emptyStream(), { event: 'steer_consumed', data: { text: 'lag tail' } }, 0);
    lag = reduceFrame(lag, { event: 'error', data: { error: 'lag', lag: 3 } }, 1);
    expect(lag.pendingEcho).toBe('lag tail');
  });

  it('withUserTurn attaches pendingEcho to the optimistic state', () => {
    const s = withUserTurn(emptyStream(), 'optimistic');
    expect(s.pendingEcho).toBe('optimistic');
    // Empty text keeps the state untouched (no anchor to remember).
    expect(withUserTurn(emptyStream(), '').pendingEcho).toBe(null);
  });
});

describe('ensurePendingEcho (reset rebuild re-push, TUI rebuild_after_reset)', () => {
  it('appends the echo when the snapshot lacks it', () => {
    const turns = [
      { kind: 'text', role: 'user', text: 'old prompt' },
      { kind: 'text', role: 'assistant', text: '压缩后的上下文' },
    ];
    const out = ensurePendingEcho(turns, '收尾总结');
    expect(out).toHaveLength(3);
    expect(out[2]).toEqual({ kind: 'text', role: 'user', text: '收尾总结' });
    // Pure: the input turns array is left untouched.
    expect(turns).toHaveLength(2);
  });

  it('appends the echo when the rebuilt turns have no user turn at all', () => {
    expect(ensurePendingEcho([{ kind: 'text', role: 'assistant', text: 'ok' }], 'tail'))
      .toEqual([
        { kind: 'text', role: 'assistant', text: 'ok' },
        { kind: 'text', role: 'user', text: 'tail' },
      ]);
  });

  it('is a no-op when the last user turn already carries the echo', () => {
    const turns = [
      { kind: 'text', role: 'assistant', text: 'ok' },
      { kind: 'text', role: 'user', text: 'same tail' },
    ];
    expect(ensurePendingEcho(turns, 'same tail')).toBe(turns);
  });

  it('is a no-op for an empty (or whitespace) echo', () => {
    const turns = [{ kind: 'text', role: 'user', text: 'hi' }];
    expect(ensurePendingEcho(turns, '')).toBe(turns);
    expect(ensurePendingEcho(turns, '   ')).toBe(turns);
  });
});

describe('applySeq watermark (resync dedup)', () => {
  it('drops a seq-carrying frame at/below applySeq, folds and stamps above it; live frames never move it', () => {
    let s = emptyStream();
    expect(s.applySeq).toBe(null);
    // Replayed frame with seq folds and sets the watermark.
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'a' }, seq: 12 }, 1);
    expect(s.applySeq).toBe(12);
    expect(s.turns.at(-1).text).toBe('a');
    // Same-seq repeat (replay overlap): dropped verbatim.
    const dup = reduceFrame(s, { event: 'text_delta', data: { text: 'b' }, seq: 12 }, 2);
    expect(dup).toBe(s);
    // Below-watermark frame: dropped even further back.
    expect(reduceFrame(s, { event: 'text_delta', data: { text: 'c' }, seq: 11 }, 3)).toBe(s);
    // Above-watermark frame folds and advances.
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'd' }, seq: 13 }, 4);
    expect(s.applySeq).toBe(13);
    expect(s.turns.at(-1).text).toBe('ad');
    // Live broadcast (no seq — emitted before the flusher persists): always
    // folds, watermark untouched.
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'e' } }, 5);
    expect(s.applySeq).toBe(13);
    expect(s.turns.at(-1).text).toBe('ade');
  });

  it('a state without applySeq (nested child folds) folds seq-carrying frames freely', () => {
    // subagent_child recursion builds bare { turns, usage, status, error }
    // states — undefined watermark must not drop anything.
    const bare = { turns: [], usage: null, status: 'streaming', error: null };
    const s = reduceFrame(bare, { event: 'text_delta', data: { text: 'x' }, seq: 7 }, 1);
    expect(s.turns.at(-1).text).toBe('x');
    expect(s.applySeq).toBe(7);
  });
});

describe('resyncState (snapshot rebuild at the watermark)', () => {
  const messages = () => [
    { role: 'user', blocks: [{ type: 'text', text: '跑测试' }] },
    { role: 'assistant', blocks: [{ type: 'text', text: '快照真相' }] },
  ];

  it('rebuilds turns from the snapshot at headSeq and keeps a draining run streaming with its pending echo re-pushed', () => {
    // Snapshot lacks the admitted turn's echo (the run is mid-flight, its
    // user message not yet visible to this read): ensurePendingEcho
    // re-pushes the user boundary so the replayed tail keeps its anchor.
    const s = resyncState({ messages: [], draining: true, headSeq: 30, pendingEcho: '跑测试' });
    expect(s.status).toBe('streaming');
    expect(s.applySeq).toBe(30);
    expect(s.pendingEcho).toBe('跑测试');
    expect(s.turns.map((t) => t.text)).toEqual(['跑测试']);
    // Frames at/below the watermark can never double-fold into the rebuild.
    expect(reduceFrame(s, { event: 'text_delta', data: { text: '旧帧' }, seq: 30 }, 1)).toBe(s);
  });

  it('a finished run (draining false) lands terminal done with the echo consumed', () => {
    const s = resyncState({ messages: messages(), draining: false, headSeq: 31, pendingEcho: '跑测试' });
    expect(s.status).toBe('done');
    expect(s.pendingEcho).toBe(null);
    expect(s.turns.map((t) => t.text)).toEqual(['跑测试', '快照真相']);
    expect(s.applySeq).toBe(31);
  });

  it('a missing headSeq leaves the watermark unset (no frame is below nothing)', () => {
    const s = resyncState({ messages: [], draining: true });
    expect(s.applySeq).toBe(null);
    expect(s.turns).toEqual([]);
  });
});
