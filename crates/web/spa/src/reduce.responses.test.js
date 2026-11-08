import { expect, it } from 'vitest';
import { emptyStream, reduceFrame } from './reduce.js';

const fold = (state, event, data = {}) => reduceFrame(state, { event, data }, 1000);

it('retry restores the round boundary while preserving completed tools', () => {
  let state = reduceFrame(emptyStream(), { event: 'steer_consumed', data: { text: 'fix it' } }, 0);
  state = fold(state, 'tool_start', { id: 'read', name: 'read', input: { path: 'a' } });
  state = fold(state, 'tool_end', { id: 'read', name: 'read', output: 'saved contents', is_error: false });
  const before = state.turns;
  state = fold(state, 'llm_round_start', { started_at_ms: 1000 });
  state = fold(state, 'reasoning_delta', { text: 'discard thinking' });
  state = fold(state, 'text_delta', { text: 'discard text' });
  // The discarded attempt streams in the live region only: its partial Say
  // sits in pendingSay (never in turns), its reasoning in the ladder.
  expect(state.pendingSay).toBe('discard text');
  expect(JSON.stringify(state.turns)).toContain('discard thinking');
  state = fold(state, 'llm_attempt_reset');
  // The discarded attempt's partial Say vanishes with the rollback.
  expect(state.pendingSay).toBe('');
  expect(state.turns).toEqual(before);
  state = fold(state, 'text_delta', { text: 'fresh answer' });
  expect(state.pendingSay).toBe('fresh answer');
  expect(JSON.stringify(state.turns)).not.toContain('fresh answer');
  state = fold(state, 'llm_round_end');
  // Round end seals the sub-turn: the fresh Say lands in the transcript.
  expect(state.pendingSay).toBe('');
  expect(JSON.stringify(state.turns)).not.toContain('discard');
  expect(JSON.stringify(state.turns)).toContain('saved contents');
  expect(JSON.stringify(state.turns)).toContain('fresh answer');
  expect(state.attemptTurns).toBeNull();
});

it('nested child retries retain their snapshot across SSE frames', () => {
  let state = fold(emptyStream(), 'subagent_start', { id: 'task', kind: 'explore' });
  const child = (event) => { state = fold(state, 'subagent_child', { id: 'task', event }); };
  child({ LlmRoundStart: { started_at_ms: 1000 } });
  child({ TextDelta: 'discard child output' });
  // The child's streaming Say rides on the subagent turn (pendingSay), so
  // it survives across frames instead of leaking into its transcript.
  expect(state.turns[0].pendingSay).toBe('discard child output');
  expect(JSON.stringify(state.turns)).toContain('discard child output');
  child('LlmAttemptReset');
  expect(state.turns[0].pendingSay).toBe('');
  child({ TextDelta: 'fresh child answer' });
  expect(state.turns[0].pendingSay).toBe('fresh child answer');
  child('LlmRoundEnd');
  expect(state.turns[0].pendingSay).toBe('');
  expect(JSON.stringify(state.turns)).not.toContain('discard child output');
  expect(JSON.stringify(state.turns)).toContain('fresh child answer');
});
