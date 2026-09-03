// reduce.order.test.js — locks the COMPOSITION invariant at the frame-folding
// layer: on the first non-empty assistant text chunk (the Say), the active
// steps ladder is settled/frozen BEFORE the Say turn is appended (foldFrame's
// text_delta case: settleTurnProgress wraps appendDelta). The steps-reducer
// layer locks the same contract in steps/reducer.test.js (c2d)/(d2); this
// file proves the two compose through the public reduceFrame export. Every
// frame below carries NO seq — live broadcasts always fold (applySeq guard).
import { describe, expect, it } from 'vitest';
import { emptyStream, reduceFrame } from './reduce.js';

// The live wire prefix of one sub-turn: user echo -> reasoning -> call ->
// result. The steps turn is armed (progressActive true) with the call
// finished — exactly the state the first Say chunk must settle.
function subTurnArmed() {
  let s = emptyStream();
  s = reduceFrame(s, { event: 'queue_consumed', data: { text: 'plan the work' } }, 1);
  s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'need the tree' } }, 2);
  s = reduceFrame(s, { event: 'tool_start', data: { id: 'a', name: 'bash', input: { cmd: 'ls' } } }, 3);
  s = reduceFrame(s, { event: 'tool_end', data: { id: 'a', name: 'bash', output: 'src target', is_error: false } }, 4);
  return s;
}

describe('reduce ordering (settle-before-append at the folding layer)', () => {
  it('(order-a) the first Say chunk settles the ladder BEFORE appending the say turn', () => {
    const armed = subTurnArmed();
    expect(armed.turns.map((t) => t.kind)).toEqual(['text', 'steps']);
    expect(armed.turns[1].progressActive).toBe(true);
    const s = reduceFrame(armed, { event: 'text_delta', data: { text: 'Done. ' } }, 5);
    // (i) steps BEFORE say in the settled order [.., steps, say].
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    expect(s.turns[1]).toMatchObject({ kind: 'steps', role: 'assistant' });
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Done. ' });
    // (ii) the frozen ladder's progress flag is already cleared when the
    // say lands, and its call is finished (in-progress cleared by tool_end).
    expect(s.turns[1].progressActive).toBe(false);
    expect(s.turns[1].steps).toHaveLength(1);
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'need the tree' });
    expect(s.turns[1].steps[0].calls[0]).toMatchObject({ id: 'a', output: 'src target', isError: false });
    // seq-less frames always fold and never move the watermark.
    expect(s.applySeq).toBe(null);
  });

  it('(order-b) a continuation chunk extends the open say and never re-touches the frozen ladder', () => {
    let s = subTurnArmed();
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Done. ' } }, 5);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'the tree is flat.' } }, 6);
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Done. the tree is flat.' });
    expect(s.turns[2].open).toBe(true);
    expect(s.turns[1].progressActive).toBe(false);
    expect(s.turns[1].steps[0].calls.map((c) => c.id)).toEqual(['a']);
  });

  it('(order-c) post-Say reasoning opens a FRESH ladder below the say, never merging into the frozen one', () => {
    let s = subTurnArmed();
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Done. ' } }, 5);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'the tree is flat.' } }, 6);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'round two' } }, 7);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'b', name: 'read', input: { path: 'x' } } }, 8);
    // (iii) fresh ladder BELOW the say: [user, steps(frozen), say, steps(new)].
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text', 'steps']);
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'Done. the tree is flat.' });
    expect(s.turns[3]).toMatchObject({ kind: 'steps', role: 'assistant', progressActive: true });
    expect(s.turns[3].steps).toHaveLength(1);
    expect(s.turns[3].steps[0]).toMatchObject({ thinking: 'round two' });
    expect(s.turns[3].steps[0].calls.map((c) => c.id)).toEqual(['b']);
    // The frozen ladder above stays untouched: still one step, one call.
    expect(s.turns[1].progressActive).toBe(false);
    expect(s.turns[1].steps).toHaveLength(1);
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'need the tree' });
    expect(s.turns[1].steps[0].calls.map((c) => c.id)).toEqual(['a']);
  });
});
