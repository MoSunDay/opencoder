import { describe, expect, it } from 'vitest';
import { initialExecutionTranscript, reduceExecutionFrame } from './transcript.js';
import { itemsFromTurns } from '../../bubbleItems.js';

describe('execution replay and continuation', () => {
  it('keeps multiple Say/Step pairs past historical done and error boundaries', () => {
    const frames = [
      ['reasoning_delta', { text: 'first thought' }], ['text_delta', { text: 'first answer' }], ['done', {}],
      ['reasoning_delta', { text: 'second thought' }], ['tool_start', { id: 'a', name: 'bash', input: { command: 'pwd' } }],
      ['tool_end', { id: 'a', output: '/repo', is_error: false }], ['text_delta', { text: 'second answer' }], ['error', { error: 'failed' }],
      ['reasoning_delta', { text: 'third thought' }], ['text_delta', { text: 'third answer' }], ['done', {}],
    ];
    const state = frames.reduce((old, [event, data], index) => reduceExecutionFrame(old, { event, data, seq: index + 1 }, index), initialExecutionTranscript());
    const turns = itemsFromTurns(state.turns).filter((item) => item.role === 'assistantTurn');
    expect(turns.map((item) => item.content.steps.length)).toEqual([1, 1, 1]);
    expect(turns.map((item) => item.content.say[0].text)).toEqual(['first answer', 'second answer', 'third answer']);
    expect(turns[1].content.steps[0].calls[0].output).toBe('/repo');
    expect(state.status).toBe('done');
    expect(reduceExecutionFrame(state, { event: 'text_delta', seq: 10, data: { text: 'third answer' } }, 20)).toBe(state);
  });
  it('marks continued activity streaming after a completed turn', () => {
    const state = reduceExecutionFrame(initialExecutionTranscript(), { event: 'done', seq: 1 }, 0);
    expect(reduceExecutionFrame(state, { event: 'reasoning_delta', seq: 2, data: { text: 'next' } }, 1).status).toBe('streaming');
  });
});
