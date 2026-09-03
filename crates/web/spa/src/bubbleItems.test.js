// bubbleItems.test.js — pure-node mapping rules for the Bubble.List migration
// (no DOM, no JSX). Guards the turn→item contract that transcript.jsx renders
// and the usage footer text that DOM smoke tests assert against.

import { describe, expect, it } from 'vitest';
import { isEmptyTranscript, itemsFromTurns, roleOfTurn, usageLine } from './bubbleItems.js';

describe('itemsFromTurns', () => {
  it('maps a mixed transcript to items with the expected role sequence', () => {
    const turns = [
      { kind: 'text', role: 'user', text: '帮我看看' },
      { kind: 'text', role: 'assistant', text: '好的' },
      { kind: 'think', role: 'assistant', text: '理解需求…' },
      { kind: 'tool', role: 'assistant', name: 'bash', input: 'ls', output: 'x', isError: false, durationMs: 900 },
      { kind: 'sys', text: 'status: running' },
    ];
    const items = itemsFromTurns(turns);
    expect(items).toHaveLength(5);
    expect(items.map((i) => i.role)).toEqual(['user', 'ai', 'think', 'tool', 'sys']);
    // The whole turn object travels as content — contentRender unpacks it.
    expect(items[3].content.name).toBe('bash');
    expect(items[4].content.text).toBe('status: running');
  });

  it('generates stable keys for the same input, unique within one list', () => {
    const turns = [
      { kind: 'text', role: 'user', text: 'a' },
      { kind: 'tool', name: 't1' },
      { kind: 'tool', name: 't2' },
      { kind: 'sys', text: 's' },
    ];
    const first = itemsFromTurns(turns).map((i) => i.key);
    const second = itemsFromTurns(turns).map((i) => i.key);
    expect(first).toEqual(second);
    expect(new Set(first).size).toBe(first.length);
    expect(first[1]).not.toBe(first[2]); // same-kind neighbours stay distinct
  });

  it('keeps an open text turn producing an ai item', () => {
    const turns = [{ kind: 'text', role: 'assistant', text: 'streaming…', open: true }];
    const items = itemsFromTurns(turns);
    expect(items).toHaveLength(1);
    expect(items[0].role).toBe('ai');
    expect(items[0].content.open).toBe(true);
  });

  it('wraps a steps segment as one visual assistant Turn', () => {
    expect(roleOfTurn({ kind: 'steps' })).toBe('steps');
    expect(roleOfTurn({ kind: 'steps', role: 'assistant' })).toBe('steps');
    const turn = {
      kind: 'steps',
      role: 'assistant',
      steps: [{ thinking: 'round one', calls: [{ kind: 'tool', name: 'bash', output: null, isError: false }] }],
    };
    const items = itemsFromTurns([{ kind: 'text', role: 'user', text: 'go' }, turn]);
    expect(items).toHaveLength(2);
    expect(items[0].role).toBe('user');
    expect(items[1].role).toBe('assistantTurn');
    expect(items[1].key).toBe('assistant-turn:1');
    expect(items[1].content.steps).toEqual(turn.steps);
    expect(items[1].content.say).toEqual([]);
  });

  it('groups adjacent steps and Say into one assistant Turn item', () => {
    const steps = { kind: 'steps', role: 'assistant', progressActive: true, steps: [] };
    const say = { kind: 'text', role: 'assistant', text: 'done' };
    const items = itemsFromTurns([steps, say]);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ role: 'assistantTurn', key: 'assistant-turn:0' });
    expect(items[0].content.steps).toEqual([]);
    expect(items[0].content.say).toEqual([say]);
    expect(items[0].content.progressActive).toBe(false);
  });

  it('merges multiple reducer step segments into one Turn step count', () => {
    const a = { kind: 'steps', role: 'assistant', progressActive: false, steps: [{ thinking: 'a', calls: [] }] };
    const mid = { kind: 'text', role: 'assistant', text: 'working' };
    const b = { kind: 'steps', role: 'assistant', progressActive: true, steps: [{ thinking: 'b', calls: [] }] };
    const done = { kind: 'text', role: 'assistant', text: 'done' };
    const items = itemsFromTurns([a, mid, b, done]);
    expect(items).toHaveLength(1);
    expect(items[0].content.steps).toEqual([...a.steps, ...b.steps]);
    expect(items[0].content.say).toEqual([mid, done]);
    expect(items[0].content.progressActive).toBe(false);
  });

  it('maps an empty-text think turn and a failed tool turn unchanged', () => {
    const turns = [
      { kind: 'think', role: 'assistant', text: '' },
      { kind: 'tool', role: 'assistant', name: 'result', input: null, output: 'boom', isError: true, durationMs: null },
    ];
    const items = itemsFromTurns(turns);
    expect(items.map((i) => i.role)).toEqual(['think', 'tool']);
    expect(items[0].content.text).toBe('');
    expect(items[1].content.isError).toBe(true);
    expect(items[1].content.durationMs).toBeNull();
  });

  it('degrades gracefully on null/undefined turns', () => {
    expect(itemsFromTurns(null)).toEqual([]);
    expect(itemsFromTurns(undefined)).toEqual([]);
    expect(roleOfTurn(null)).toBe('ai');
    expect(roleOfTurn({})).toBe('ai');
  });
});

describe('usageLine', () => {
  it('formats a complete usage frame like the old UsageFooter', () => {
    expect(usageLine({ input: 10, output: 5, total: 15, contextWindow: 100 }))
      .toBe('▲ in 10  ▼ out 5  Σ 15 · 上下文 15%');
  });

  it('omits the context clause when contextWindow is missing', () => {
    expect(usageLine({ input: 1, output: 2, total: 3 })).toBe('▲ in 1  ▼ out 2  Σ 3');
    expect(usageLine({ input: 1, output: 2, total: 3, contextWindow: 0 }))
      .toBe('▲ in 1  ▼ out 2  Σ 3');
  });

  it('fills 0 placeholders for missing fields and null usage', () => {
    expect(usageLine({})).toBe('▲ in 0  ▼ out 0  Σ 0');
    expect(usageLine({ input: 7 })).toBe('▲ in 7  ▼ out 0  Σ 0');
    expect(usageLine(null)).toBe('▲ in 0  ▼ out 0  Σ 0');
    expect(usageLine(undefined)).toBe('▲ in 0  ▼ out 0  Σ 0');
  });

  it('caps the context percentage at 999%', () => {
    const line = usageLine({ input: 99999, output: 0, total: 99999, contextWindow: 10 });
    expect(line).toContain('上下文 999%');
  });
});

describe('isEmptyTranscript', () => {
  it('is empty only without turns and without usage', () => {
    expect(isEmptyTranscript([], null)).toBe(true);
    expect(isEmptyTranscript(undefined, null)).toBe(true);
    expect(isEmptyTranscript([], {})).toBe(false); // usage chip still shows
    expect(isEmptyTranscript([{ kind: 'sys', text: 'x' }], null)).toBe(false);
  });
});
