import { expect, it } from 'vitest';
import { moveInitiative } from './model.js';

it('moves in either direction and preserves hidden initiative order without mutating input', () => {
  const rows = ['a', 'hidden1', 'hidden2', 'b'].map((id) => ({ id }));
  expect(moveInitiative(rows, 'a', 'b').map((r) => r.id)).toEqual(['hidden1', 'hidden2', 'b', 'a']);
  expect(moveInitiative(rows, 'b', 'a').map((r) => r.id)).toEqual(['b', 'a', 'hidden1', 'hidden2']);
  expect(rows.map((r) => r.id)).toEqual(['a', 'hidden1', 'hidden2', 'b']);
  expect(moveInitiative(rows, 'a', 'a')).toBeNull();
  expect(moveInitiative(rows, 'missing', 'a')).toBeNull();
  expect(moveInitiative(rows, 'a', 'missing')).toBeNull();
});
