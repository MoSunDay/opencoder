// Frame-level e2e for the "N Steps + Say" transcript contract (TUI parity,
// features/changelog/2026-09-03/say-pairs-steps-all-surfaces.md and
// say-closes-turn-transcript-reset-echo.md): one user input owns one or MORE
// [n Steps + Say] pairs; every non-empty Say CLOSES its sub-turn and the next
// reasoning/tool run opens a FRESH ladder strictly below that Say.
//
// These tests drive the LIVE wire path only: every frame goes through the
// public reduceFrame export from an emptyStream start, with NO `seq` on any
// frame (live broadcasts are emitted before the async flusher persists them,
// so they never carry one - see reduce.js). The user echo of the admitted
// prompt rides in via withUserTurn, exactly like chat.jsx's optimistic echo
// (a fresh local run emits no steer/queue echo frame). Real-browser
// verification stays manual; this file locks the fold semantics.
import { describe, expect, it } from 'vitest';
import { emptyStream, ensurePendingEcho, reduceFrame, turnsFromMessages, withUserTurn } from './reduce.js';

const callIds = (ladder) => ladder.steps.flatMap((step) => step.calls.map((call) => call.id));
const callCount = (ladder) => ladder.steps.reduce((n, step) => n + step.calls.length, 0);
const stepCallCounts = (ladder) => ladder.steps.map((step) => step.calls.length);
const indexOfKind = (turns, kind, from = 0) => turns.findIndex((t, i) => i >= from && t.kind === kind);

describe('(a) two rounds in one run alternate [N Steps + Say] x 2', () => {
  it('settles [user, steps(2 calls), say, steps(1 call), say] with both ladders frozen by their own Say', () => {
    let s = withUserTurn(emptyStream(), 'ship the release');
    expect(s.pendingEcho).toBe('ship the release');

    // Round 1: thinking -> call+result -> thinking -> call+result.
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'inspect the tree' } }, 1000);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'r1a', name: 'bash', input: { cmd: 'ls' } } }, 1010);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'r1a', name: 'bash', output: 'a.txt', is_error: false, duration_ms: 120 } }, 1130);
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'verify the fix' } }, 1200);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'r1b', name: 'read', input: { path: 'src/lib.rs' } } }, 1210);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'r1b', name: 'read', output: 'fn main() {}', is_error: false, duration_ms: 80 } }, 1290);
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps']);

    // Say 1 streams in chunks: its FIRST chunk freezes the ladder it closes
    // AND hands the running hint to the Say row (sayStreaming) — the ladder
    // stays "running" through the whole Say even though progress froze.
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'First round done.' } }, 1300);
    const ladder1 = s.turns[1];
    expect(ladder1.progressActive).toBe(false);
    expect(ladder1.sayStreaming).toBe(true);
    s = reduceFrame(s, { event: 'text_delta', data: { text: ' Two calls used.' } }, 1310);
    expect(s.turns[2]).toMatchObject({ kind: 'text', role: 'assistant', text: 'First round done. Two calls used.' });
    // Continuation chunks keep the flag on the same (pair-1) ladder.
    expect(s.turns[1].sayStreaming).toBe(true);

    // Round 2 opens a FRESH ladder strictly BELOW Say 1, re-armed. The new
    // ladder activity retires pair-1's Say-row running hint.
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'tighten up' } }, 1400);
    expect(s.turns[1].sayStreaming).toBe(false);
    expect(s.turns[3].sayStreaming).toBeFalsy();
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'r2a', name: 'bash', input: { cmd: 'cargo test' } } }, 1410);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'r2a', name: 'bash', output: '12 passed', is_error: false, duration_ms: 900 } }, 2310);
    const ladder2 = s.turns[3];
    expect(ladder2.progressActive).toBe(true);

    // Say 2 freezes the round-2 ladder exactly when it starts streaming —
    // and takes over the running hint for pair 2.
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'All green.' } }, 2320);
    expect(s.turns[3].sayStreaming).toBe(true);
    s = reduceFrame(s, { event: 'done', data: {} }, 2330);

    // Settled transcript: [user, steps, say, steps, say].
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text', 'steps', 'text']);
    expect(s.status).toBe('done');
    expect(s.pendingEcho).toBe(null);
    expect(s.turns.every((t) => t.kind !== 'think')).toBe(true);

    expect(s.turns[0]).toMatchObject({ kind: 'text', role: 'user', text: 'ship the release' });

    // EXACT per-ladder call counts: 2 then 1, with the round split per step.
    expect(callCount(s.turns[1])).toBe(2);
    expect(callIds(s.turns[1])).toEqual(['r1a', 'r1b']);
    expect(stepCallCounts(s.turns[1])).toEqual([1, 1]);
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'inspect the tree' });
    expect(s.turns[1].steps[1]).toMatchObject({ thinking: 'verify the fix' });
    expect(s.turns[1].steps[0].calls[0]).toMatchObject({ output: 'a.txt', isError: false, durationMs: 120 });
    expect(s.turns[1].steps[1].calls[0]).toMatchObject({ output: 'fn main() {}', isError: false, durationMs: 80 });

    expect(callCount(s.turns[3])).toBe(1);
    expect(callIds(s.turns[3])).toEqual(['r2a']);
    expect(s.turns[3].steps[0]).toMatchObject({ thinking: 'tighten up' });
    expect(s.turns[3].steps[0].calls[0]).toMatchObject({ output: '12 passed', isError: false, durationMs: 900 });

    // Both ladders frozen after their own Say; the trailing Say is closed by
    // done (a mid Say keeps open:true until something re-closes the tail).
    expect(s.turns[1].progressActive).toBe(false);
    expect(s.turns[3].progressActive).toBe(false);
    expect(s.turns[4]).toMatchObject({ kind: 'text', role: 'assistant', text: 'All green.', open: false });
    // Say-row running per pair: pair 2's Say activated its own flag (asserted
    // right after its first chunk), and the terminal done retired BOTH hints
    // — no running survives the terminal boundary.
    expect(s.turns[1].sayStreaming).toBe(false);
    expect(s.turns[3].sayStreaming).toBe(false);

    // The second ladder sits strictly BELOW the first Say (index order).
    const say1 = indexOfKind(s.turns, 'text', 1);
    expect(say1).toBe(2);
    expect(indexOfKind(s.turns, 'steps', say1 + 1)).toBeGreaterThan(say1);
    expect(indexOfKind(s.turns, 'steps', say1 + 1)).toBe(3);
  });
});

describe('(b) transcript_reset keeps the in-flight user echo', () => {
  // Post-clear store snapshot: ClearContext folds the transcript to a
  // continuity-seed assistant message. The compound tail of the in-flight
  // `/act_clear_context <tail>` is recorded only AFTER the reset fired, so
  // the snapshot still lacks the echo (TUI rebuild_after_reset parity).
  const collapsedSnapshot = () => [
    { role: 'assistant', blocks: [{ kind: 'text', text: '[compacted continuity seed]' }] },
  ];

  it('preserves pendingEcho across the reset frame and the snapshot rebuild re-pushes the echo turn', () => {
    let s = withUserTurn(emptyStream(), 'fix the login bug');
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'repro first' } }, 1000);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'c1', name: 'bash', input: { cmd: 'curl -i' } } }, 1010);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'c1', name: 'bash', output: 'HTTP/1.1 401', is_error: false, duration_ms: 250 } }, 1260);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Draft answer before the wipe.' } }, 1270);
    expect(s.pendingEcho).toBe('fix the login bug');

    // Mid-run `/act_clear_context finish the summary`: the runner echoes the
    // compound TAIL as steer_consumed (consumedEchoText strips the command
    // head), then ClearContext emits transcript_reset with an empty payload
    // (runner/event.rs serializes TranscriptReset as {} on the wire).
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: 'finish the summary' } }, 1300);
    s = reduceFrame(s, { event: 'transcript_reset', data: {} }, 1310);

    // Frame-level guarantees: the echo turn landed, the pre-reset Say got its
    // open flag closed, and the reset INTENTIONALLY kept the echo memory.
    expect(s.turns.at(-1)).toMatchObject({ kind: 'text', role: 'user', text: 'finish the summary' });
    expect(s.turns.find((t) => t.text === 'Draft answer before the wipe.').open).toBe(false);
    expect(s.pendingEcho).toBe('finish the summary');

    // chat.jsx rebuilds on transcript_reset via
    // ensurePendingEcho(turnsFromMessages(snapshot), pendingEcho): the prior
    // assistant content is dropped with the collapse, the in-flight echo is
    // re-pushed as the transcript's user boundary.
    const rebuilt = ensurePendingEcho(turnsFromMessages(collapsedSnapshot()), s.pendingEcho);
    expect(rebuilt.some((t) => t.text === 'Draft answer before the wipe.')).toBe(false);
    expect(rebuilt.some((t) => t.kind === 'steps')).toBe(false);
    expect(rebuilt.map((t) => t.kind)).toEqual(['text', 'text']);
    expect(rebuilt[0]).toMatchObject({ kind: 'text', role: 'assistant', text: '[compacted continuity seed]' });
    expect(rebuilt[1]).toMatchObject({ kind: 'text', role: 'user', text: 'finish the summary' });
    // Idempotent re-push: a second rebuild never duplicates the boundary.
    expect(ensurePendingEcho(rebuilt, s.pendingEcho)).toBe(rebuilt);
  });

  it('done retires the echo so a later bare reset cannot resurrect the old prompt', () => {
    let s = withUserTurn(emptyStream(), 'fix the login bug');
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'partial say' } }, 1000);
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: 'finish the summary' } }, 1010);
    s = reduceFrame(s, { event: 'transcript_reset', data: {} }, 1020);
    expect(s.pendingEcho).toBe('finish the summary');

    s = reduceFrame(s, { event: 'done', data: {} }, 1030);
    expect(s.status).toBe('done');
    expect(s.pendingEcho).toBe(null);

    // A later BARE reset (e.g. a second clear) rebuilds from the snapshot
    // with no echo memory: the previous turn's prompt must not resurface.
    const later = reduceFrame(s, { event: 'transcript_reset', data: {} }, 1040);
    expect(later.pendingEcho).toBe(null);
    const rebuilt = ensurePendingEcho(turnsFromMessages(collapsedSnapshot()), later.pendingEcho);
    expect(rebuilt.some((t) => t.role === 'user' && t.text === 'finish the summary')).toBe(false);
    expect(rebuilt.some((t) => t.role === 'user' && t.text === 'fix the login bug')).toBe(false);
  });
});

describe('(c) steer consumption opens a fresh ladder', () => {
  it('anchors the new ladder below the steer echo and leaves the round-1 ladder untouched', () => {
    let s = withUserTurn(emptyStream(), 'audit the module');
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'scan first' } }, 1000);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'c1', name: 'bash', input: { cmd: 'grep -r TODO' } } }, 1010);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'c1', name: 'bash', output: '3 hits', is_error: false, duration_ms: 60 } }, 1070);
    s = reduceFrame(s, { event: 'text_delta', data: { text: 'Initial findings are in.' } }, 1080);

    // Round-1 Say closed its ladder: snapshot the frozen group. The Say row
    // carries the running hint (sayStreaming) while its sub-turn is the
    // live one — the steer boundary below ends it.
    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text']);
    const ladder1 = s.turns[1];
    expect(ladder1.progressActive).toBe(false);
    expect(ladder1.sayStreaming).toBe(true);

    // Mid-run steer: steer_consumed pushes the user echo (a hard segment
    // boundary - it also closes the previous Say) and remembers pendingEcho.
    s = reduceFrame(s, { event: 'steer_consumed', data: { text: 'also check the error paths' } }, 1100);
    expect(s.pendingEcho).toBe('also check the error paths');
    expect(s.turns.find((t) => t.text === 'Initial findings are in.').open).toBe(false);
    // The boundary settles the ladder terminal-side: the round-1 Say row's
    // running hint retires with it (only field ever touched — everything
    // else stays deep-equal to the frozen snapshot).
    expect(s.turns[1].sayStreaming).toBe(false);

    // Round 2 reasoning + call: the user boundary CAPS the backwards walk -
    // nothing merges into the closed group above Say 1.
    s = reduceFrame(s, { event: 'reasoning_delta', data: { text: 'round two reasoning' } }, 1110);
    s = reduceFrame(s, { event: 'tool_start', data: { id: 'c2', name: 'bash', input: { cmd: 'cargo test' } } }, 1120);
    s = reduceFrame(s, { event: 'tool_end', data: { id: 'c2', name: 'bash', output: 'ok', is_error: false, duration_ms: 40 } }, 1160);

    expect(s.turns.map((t) => t.kind)).toEqual(['text', 'steps', 'text', 'text', 'steps']);
    // The steer echo turn, then the NEW ladder strictly below it.
    expect(s.turns[3]).toMatchObject({ kind: 'text', role: 'user', text: 'also check the error paths' });
    const ladder2 = s.turns[4];
    expect(indexOfKind(s.turns, 'steps', 3)).toBeGreaterThan(3);
    expect(ladder2.steps).toHaveLength(1);
    expect(ladder2.steps[0]).toMatchObject({ thinking: 'round two reasoning' });
    expect(callIds(ladder2)).toEqual(['c2']);
    expect(ladder2.steps[0].calls[0]).toMatchObject({ output: 'ok', isError: false });
    // Re-armed: its own Say has not arrived yet.
    expect(ladder2.progressActive).toBe(true);

    // The round-1 ladder is untouched by the steer boundary beyond the
    // retired sayStreaming hint (deep-equal to the frozen snapshot; the walk
    // never reached above the user echo).
    expect(s.turns[1]).toEqual({ ...ladder1, sayStreaming: false });
    expect(s.turns[1].progressActive).toBe(false);
    expect(callIds(s.turns[1])).toEqual(['c1']);
    expect(s.turns[1].steps).toHaveLength(1);
    expect(s.turns[1].steps[0]).toMatchObject({ thinking: 'scan first' });
  });
});
