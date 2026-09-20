import { expect, it } from 'vitest';
import { batchError, dispatchInput, progressLabel } from './model.js';
import { validateSpec } from '../specValidate.js';
import { applyDagFrame } from '../run/model.js';
const step = (name, pointer, type = 'agent') => ({ name, kind: { type: 'dynamic', source: { type: 'input', pointer }, template: { type, prompt: 'common', command: 'tool.wasm' } } });
it('assembles nested batch input and preserves argv elements verbatim', () => {
  const spec = { steps: [step('a', '/items'), step('b', '/nested/a~1b', 'wasm')] };
  expect(dispatchInput(spec, { a: '["first","second"]', b: '[["--title","hello world"]]' }))
    .toEqual({ items: ['first', 'second'], nested: { 'a/b': [['--title', 'hello world']] } });
  expect(dispatchInput({ steps: [step('a', '')] }, { a: '[]' })).toEqual([]);
  const input = dispatchInput({ steps: [step('a', '/__proto__/items')] }, {});
  expect(Object.prototype.items).toBeUndefined();
  expect(JSON.parse(JSON.stringify(input))).toEqual({ __proto__: undefined, ['__proto__']: { items: [] } });
});
it('rejects invalid types, limits, and conflicting source paths before dispatch', () => {
  expect(batchError({ type: 'agent' }, ['text', 1])).toContain('实例 1');
  expect(batchError({ type: 'wasm' }, [['ok'], ['bad', 1]])).toContain('实例 1');
  expect(batchError({ type: 'agent' }, Array(1000).fill('x'))).toBe('');
  expect(batchError({ type: 'agent' }, Array(1001).fill('x'))).toContain('1,000');
  expect(() => dispatchInput({ steps: [step('a', '/items'), step('b', '/items')] }, { a: '["a"]', b: '["b"]' })).toThrow('必须一致');
  expect(() => dispatchInput({ steps: [step('a', '/items'), step('b', '/items/nested')] }, {})).toThrow('相互包含');
});
it('validates dynamic templates and requires upstream source dependencies', () => {
  const a = step('a', '/items');
  expect(validateSpec({ name: 'd', steps: [a] })).toEqual([]);
  const b = { ...step('b', '/items'), kind: { ...a.kind, source: { type: 'step_output', step: 'a', pointer: '/items' } } };
  expect(validateSpec({ name: 'd', steps: [a, b] }).join()).toContain('depends_on');
  expect(validateSpec({ name: 'd', steps: [a, { ...b, depends_on: ['a'] }] })).toEqual([]);
});
it('assigns absolute progress once and rejects stale snapshot-era frames', () => {
  const state = { head_seq: 5, steps: [{ name: 'a', status: 'running', instances_at_ms: 20, instances: { done: 2, total: 1000 } }] };
  const frame = { seq: 6, event: 'step_progress', data: { kind: 'step_progress', step: 'a', at_ms: 21, payload: { instances: { done: 3, total: 1000 } } } };
  const next = applyDagFrame(state, frame);
  expect(next.steps[0].status).toBe('running');
  expect(next.steps[0].instances.done).toBe(3);
  expect(applyDagFrame(next, frame)).toBe(next);
  expect(applyDagFrame(state, { ...frame, data: { ...frame.data, at_ms: 19 } }).steps[0].instances.done).toBe(2);
  expect(progressLabel(null)).toBe('等待派生');
  expect(progressLabel({ done: 0, total: 0 })).toBe('0/0，无需执行');
});
