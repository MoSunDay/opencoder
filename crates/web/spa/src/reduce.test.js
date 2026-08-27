// vitest smoke tests for the pure transcript reducers (reduce.js).
import { describe, expect, it } from 'vitest';
import { deltaTextOf, emptyStream, reduceFrame, turnsFromMessages, withUserTurn } from './reduce.js';

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
    expect(turns[2]).toMatchObject({ kind: 'tool', name: 'bash', output: 'a.txt', isError: false });
  });

  it('tolerates missing messages/blocks', () => {
    expect(turnsFromMessages(undefined)).toEqual([]);
    expect(turnsFromMessages([{ role: 'user', blocks: undefined }])).toEqual([]);
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
});
