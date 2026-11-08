// reduce.order.test.js — locks the COMPOSITION invariant at the frame-folding
// layer (TUI live-region parity): a STREAMING Say is NOT transcript content —
// text_delta chunks only accumulate in `pendingSay` (rendered solely in the
// turn live region) and the turn array stays untouched. The ladder settles
// AND the Say lands TOGETHER at FLUSH time (foldFrame's flushPendingSay, run
// first in the boundary arms: round end, next tool/thinking activity,
// terminal settle, new input), with the FULL text in one shot: the settled
// order is [.., steps, say] and the landed Say never takes the sayStreaming
// running hint. The steps-reducer layer locks the same contract in
// steps/reducer.test.js (c2d)/(d2); this file proves the two compose through
// the public reduceFrame export. Every frame below carries NO seq — live
// broadcasts always fold (applySeq guard).
import { describe, expect, it } from 'vitest';
import { emptyStream, reduceFrame } from './reduce.js';

// The live wire prefix of one sub-turn: user echo -> reasoning -> call ->
// result. The steps turn is armed (progressActive true) with the call
// finished — exactly the state the Say's flush must settle.
function subTurnArmed() {
  let s = emptyStream();
  s = reduceFrame(s, { event: 'queue_consumed', data: { text: 'plan the work' } }, 1);
  s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'need the tree' } }, 2);
  s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: { cmd: 'ls' } } }, 3);
  s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'src target', is_error: false } }, 4);
  return s;
}

describe('reduce ordering (settle + say landing at flush time, folding layer)', () => {
  it('(order-a) streaming chunks never touch the turns; the flush settles the ladder and lands the FULL say in one shot', () => {
    const armed = subTurnArmed();
    expect(armed.turns.map((t) => t.kind)).toEqual(['text', 'steps']);
    expect(armed.turns[1].progressActive).toBe(true);
    let s = reduceFrame(armed, { event: 'text_delta', data: { text: 'Done. ' } }, 5);
    // NOT transcript content while streaming: only pendingSay accumulates —
    // the turn array keeps its identity and the ladder keeps running.
    expect(s.pendingSay).toBe('Done. ');
    expect(s.turns).toBe(armed.turns);
    expect(s.turns[1].progressActive).toBe(true);
    // Round end flushes: settleTurnProgress and appendDelta run together.
    s = reduceFrame(s, { event: 'llm_round_end', data: {} }, 6);
    // (i) steps BEFORE say in the settled order [.., steps, say].
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    expect(s.turns[1]).toMatchObject({ kind: 'steps', role: 'assistant' });
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Done. ' });
    // (ii) the frozen ladder's progress flag is cleared by the SAME flush
    // that lands the say (in-progress was already cleared by tool_end).
    expect(s.turns[1].progressActive).toBe(false);
    // (ii-b) TUI parity: the landed Say is complete, not streaming — the
    // flush never marks sayStreaming, so no Say-row running hint appears.
    expect(s.turns[1].sayStreaming).toBeUndefined();
    expect(s.pendingSay).toBe('');
    expect(s.turns[1].steps).toHaveLength(1);
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'need the tree' });
    expect(s.turns[1].steps[0].calls[0]).toMatchObject({ id: 'a', output: 'src target', isError: false });
    // seq-less frames always fold and never move the watermark.
    expect(s.applySeq).toBe(null);
  });

  it('(order-b) continuation chunks only extend pendingSay; the flush lands the whole say once and never re-touches the frozen ladder', () => {
    let s = subTurnArmed();
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Done. ' } }, 5);
    const mid = reduceFrame(s, { event: 'text_delta', data: { text: 'the tree is flat.' } }, 6);
    expect(mid.pendingSay).toBe('Done. the tree is flat.');
    expect(mid.turns).toBe(s.turns);
    expect(mid.turns[1].progressActive).toBe(true);
    s = reduceFrame(mid, { event: 'llm_round_end', data: {} }, 7);
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Done. the tree is flat.' });
    expect(s.turns[2].open).toBe(true);
    expect(s.turns[1].progressActive).toBe(false);
    expect(s.turns[1].sayStreaming).toBeUndefined();
    expect(s.pendingSay).toBe('');
    expect(s.turns[1].steps[0].calls.map((c) => c.id)).toEqual(['a']);
  });

  it('(order-c) post-Say reasoning flushes the say FIRST, then opens a FRESH ladder below it, never merging into the frozen one', () => {
    let s = subTurnArmed();
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Done. ' } }, 5);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'the tree is flat.' } }, 6);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'round two' } }, 7);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: { path: 'x' } } }, 8);
    // (iii) the say landed BEFORE the new ladder: [user, steps(frozen), say, steps(new)].
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text', 'steps']);
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Done. the tree is flat.' });
    expect(s.turns[3]).toMatchObject({ kind: 'steps', role: 'assistant', progressActive: true });
    expect(s.turns[3].steps).toHaveLength(1);
    expect(s.turns[3].steps[0]).toMatchObject({ thinking: 'round two' });
    expect(s.turns[3].steps[0].calls.map((c) => c.id)).toEqual(['b']);
    expect(s.pendingSay).toBe('');
    // The frozen ladder above stays untouched: still one step, one call —
    // and no Say-row running hint anywhere (the landed Say never takes one).
    expect(s.turns[1].progressActive).toBe(false);
    expect(s.turns[1].sayStreaming).toBeUndefined();
    expect(s.turns[3].sayStreaming).toBeFalsy();
    expect(s.turns[1].steps).toHaveLength(1);
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'need the tree' });
    expect(s.turns[1].steps[0].calls.map((c) => c.id)).toEqual(['a']);
  });
});
