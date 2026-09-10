import { expect, it } from 'vitest';
import { emptyStream, reduceFrame, withUserTurn } from './reduce.js';

const fold = (state, event, data = {}) => reduceFrame(state, { event, data }, 1000);

it('retry restores the round boundary while preserving completed tools', () => {
  let state = withUserTurn(emptyStream(), 'fix it');
  state = fold(state, 'tool_start', { id: 'read', name: 'read', input: { path: 'a' } });
  state = fold(state, 'tool_end', { id: 'read', name: 'read', output: 'saved contents', is_error: false });
  const before = state.turns;
  state = fold(state, 'llm_round_start', { started_at_ms: 1000 });
  state = fold(state, 'reasoning_delta', { text: 'discard thinking' });
  state = fold(state, 'text_delta', { text: 'discard text' });
  expect(JSON.stringify(state.turns)).toContain('discard');
  state = fold(state, 'llm_attempt_reset');
  expect(state.turns).toEqual(before);
  state = fold(state, 'text_delta', { text: 'fresh answer' });
  state = fold(state, 'llm_round_end');
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
  expect(JSON.stringify(state.turns)).toContain('discard child output');
  child('LlmAttemptReset');
  child({ TextDelta: 'fresh child answer' });
  child('LlmRoundEnd');
  expect(JSON.stringify(state.turns)).not.toContain('discard child output');
  expect(JSON.stringify(state.turns)).toContain('fresh child answer');
});
