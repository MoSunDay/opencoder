import { describe, expect, it } from 'vitest';
import { emptyStream, reduceFrame, turnsFromMessages, withUserTurn } from '../reduce.js';

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

  it('(b) sequential calls without new thinking stay in ONE step', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'x', is_error: false } }, 2);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 3);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].kind).toBe('steps');
    expect(s.turns[0].steps).toHaveLength(1);
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ id: 'a', output: 'x' });
    expect(s.turns[0].steps[0].calls[1]).toMatchObject({ id: 'b', output: null });
  });

  it('(c) reasoning_delta streams straight into the ladder — a think-only step, never a top-level think turn', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'plan ' } }, 1);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'it' } }, 2);
    // The very first reasoning char already lands inside a steps turn.
    expect(s.turns.map((x) => x.kind)).toEqual(['steps']);
    expect(s.turns[0].steps[0]).toMatchObject({ thinking: 'plan it', calls: [] });
    // An empty delta is a no-op (same state object back).
    expect(reduceFrame(s, { event: 'reasoning_delta', data: { text: '' } }, 3)).toBe(s);
    // tool_start joins THAT step: thinking already there, same step, one turn.
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 4);
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].kind).toBe('steps');
    expect(s.turns[0].steps).toHaveLength(1);
    expect(s.turns[0].steps[0]).toMatchObject({ thinking: 'plan it' });
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ id: 'a', output: null });
    expect(s.turns.every((t) => t.kind !== 'think')).toBe(true);
  });

  it('(c2) think → Say → think interleave: both runs land in ONE step, the steps turn sits BEFORE the Say', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'first ' } }, 1);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'mid Say' } }, 2);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'second' } }, 3);
    // The walk rides back over the OPEN Say into the ladder behind it.
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text']);
    expect(s.turns[0].steps).toHaveLength(1);
    expect(s.turns[0].steps[0].thinking).toBe('first second');
    expect(s.turns[1]).toMatchObject({ kind: 'text', role: 'assistant', text: 'mid Say' });
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 4);
    // Say is presentation inside this Turn: the call joins the existing
    // reasoning Step and no second group is manufactured.
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text']);
    expect(s.turns[0].steps).toHaveLength(1);
    expect(s.turns[0].steps[0].thinking).toBe('first second');
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ id: 'a', output: null });
    expect(s.turns.every((t) => t.kind !== 'think')).toBe(true);
  });

  it('(c2b) reasoning after a finished round opens a NEW think-only step in the same ladder', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'x', is_error: false } }, 2);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'next round' } }, 3);
    expect(s.turns.map((x) => x.kind)).toEqual(['steps']);
    expect(s.turns[0].steps).toHaveLength(2);
    expect(s.turns[0].steps[0].calls[0]).toMatchObject({ id: 'a', output: 'x' });
    expect(s.turns[0].steps[1]).toMatchObject({ thinking: 'next round', calls: [] });
  });

  it('(c2b-2) new reasoning opens a Step even while the previous call is running', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'next thought' } }, 2);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 3);
    expect(s.turns[0].steps).toHaveLength(2);
    expect(s.turns[0].steps[0].calls.map((call) => call.id)).toEqual(['a']);
    expect(s.turns[0].steps[1]).toMatchObject({ thinking: 'next thought' });
    expect(s.turns[0].steps[1].calls.map((call) => call.id)).toEqual(['b']);
  });

  it('(c2c) reasoning after a closed boundary (tail is a user turn) inserts the steps turn right after it', () => {
    let s = withUserTurn(emptyStream(), 'do the thing');
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'fresh round' } }, 1);
    expect(s.turns.map((x) => x.kind)).toEqual(['text', 'steps']);
    expect(s.turns[0]).toMatchObject({ kind: 'text', role: 'user', text: 'do the thing' });
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'fresh round', calls: [] });
  });

  it('(c3) user boundaries split Turns while sys presentation stays inside one Turn', () => {
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
    expect(s.turns.every((t) => t.kind !== 'think')).toBe(true);
    // A sys marker is presentation inside the same admitted Turn.
    let t = emptyStream();
    t = reduceFrame(t, { event: 'reasoning_delta', data: { text: 'pre-marker' } }, 1);
    t = reduceFrame(t, { event: 'status', data: { status: 'thinking' } }, 2);
    t = reduceFrame(t, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 3);
    expect(t.turns.map((x) => x.kind)).toEqual(['steps', 'sys']);
    expect(t.turns[0].steps[0].thinking).toBe('pre-marker');
    expect(t.turns[0].steps[0].calls[0]).toMatchObject({ id: 'a' });
    expect(t.turns.every((x) => x.kind !== 'think')).toBe(true);
  });

  it('(c4) a pure-text round (no tool call ever) keeps its call-less step at done — think turn never existed', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'ponder' } }, 1);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'final answer' } }, 2);
    s = reduceFrame(s, { event: 'done', data: {} }, 3);
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text']);
    expect(s.turns[0].steps[0]).toMatchObject({ thinking: 'ponder', calls: [] });
    expect(s.turns[1]).toMatchObject({ kind: 'text', text: 'final answer' });
    expect(s.turns.every((t) => t.kind !== 'think')).toBe(true);
  });

  it('(d) Say between calls does not open a Step without new thinking', () => {
    let s = emptyStream();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 1);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'x', is_error: false } }, 2);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'interlude answer' } }, 3);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 4);
    expect(s.turns.map((x) => x.kind)).toEqual(['steps', 'text']);
    expect(s.turns[0].steps).toHaveLength(1);
    expect(s.turns[0].steps[0].calls.map((call) => call.id)).toEqual(['a', 'b']);
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

  it('(h) adjacent tool messages without new thinking fold into ONE step', () => {
    const turns = turnsFromMessages([
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'a', name: 'bash', input: {} }] },
      { role: 'assistant', blocks: [{ kind: 'tool_use', id: 'b', name: 'read', input: {} }] },
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0].kind).toBe('steps');
    expect(turns[0].steps).toHaveLength(1);
    expect(turns[0].steps[0].calls.map((call) => call.id)).toEqual(['a', 'b']);
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
    expect(turns.map((t) => t.kind)).toEqual(['steps', 'text']);
    expect(turns[0].steps[0].thinking).toBe('hmm');
    expect(turns[1]).toMatchObject({ kind: 'text', role: 'assistant', text: 'let me check' });
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
      {
        kind: 'steps', role: 'assistant', progressActive: false,
        steps: [{ thinking: 'dangling thought', calls: [] }],
      },
    ]);
  });
});

it('live SSE and snapshot replay produce the same Turn/Step/call hierarchy', () => {
  let live = withUserTurn(emptyStream(), 'go');
  live = reduceFrame(live, { event: 'reasoning_delta', data: { text: 'first' } }, 1);
  live = reduceFrame(live, { event: 'text_delta', data: { text: 'checking' } }, 2);
  live = reduceFrame(live, { event: 'tool_start', data: { id: 'a', name: 'bash', input: {} } }, 3);
  live = reduceFrame(live, { event: 'tool_start', data: { id: 'a2', name: 'write', input: {} } }, 3);
  live = reduceFrame(live, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'A' } }, 4);
  live = reduceFrame(live, { event: 'tool_end', data: { id: 'a2', name: 'write', output: 'A2' } }, 4);
  live = reduceFrame(live, { event: 'reasoning_delta', data: { text: 'second' } }, 5);
  live = reduceFrame(live, { event: 'tool_start', data: { id: 'b', name: 'read', input: {} } }, 6);
  live = reduceFrame(live, { event: 'tool_end', data: { id: 'b', name: 'read', output: 'B' } }, 7);
  live = reduceFrame(live, { event: 'text_delta', data: { text: 'done' } }, 8);

  const replay = turnsFromMessages([
    { role: 'user', blocks: [{ kind: 'text', text: 'go' }] },
    { role: 'assistant', blocks: [
      { kind: 'reasoning', text: 'first' },
      { kind: 'text', text: 'checking' },
      { kind: 'tool_use', id: 'a', name: 'bash', input: {} },
      { kind: 'tool_use', id: 'a2', name: 'write', input: {} },
    ] },
    { role: 'tool', blocks: [
      { kind: 'tool_result', tool_use_id: 'a', output: 'A' },
      { kind: 'tool_result', tool_use_id: 'a2', output: 'A2' },
    ] },
    { role: 'assistant', blocks: [
      { kind: 'reasoning', text: 'second' },
      { kind: 'tool_use', id: 'b', name: 'read', input: {} },
    ] },
    { role: 'tool', blocks: [{ kind: 'tool_result', tool_use_id: 'b', output: 'B' }] },
    { role: 'assistant', blocks: [{ kind: 'text', text: 'done' }] },
  ]);
  const hierarchy = (turns) => turns
    .filter((turn) => turn.kind === 'steps')
    .map((turn) => turn.steps.map((step) => ({
      thinking: step.thinking,
      calls: step.calls.map((call) => ({ id: call.id, name: call.name, output: call.output })),
    })));

  expect(hierarchy(live.turns)).toEqual(hierarchy(replay));
  expect(hierarchy(live.turns)).toHaveLength(1);
  expect(hierarchy(live.turns)[0]).toHaveLength(2);
});
