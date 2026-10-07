import { expect, it } from 'vitest';
import { assignmentBadge, moveTodo } from './board.js';

const rows = [
  { id: 'a', board_status: 'backlog', position: 1000 },
  { id: 'b', board_status: 'backlog', position: 2000 },
  { id: 'c', board_status: 'todo', position: 1000 },
];

it('moves a card across lanes and orders the entire destination lane', () => {
  const result = moveTodo(rows, 'b', 'c');
  expect(result.ids).toEqual(['b', 'c']);
  expect(result.preview.find((item) => item.id === 'b')).toMatchObject({ board_status: 'todo', position: 1000 });
  expect(result.preview.find((item) => item.id === 'c').position).toBe(2000);
  expect(rows[1].board_status).toBe('backlog');
});

it('reorders within a lane and refuses an unknown destination', () => {
  expect(moveTodo(rows, 'b', 'a').ids).toEqual(['b', 'a']);
  expect(moveTodo(rows, 'b', 'lane:done').ids).toEqual(['b']);
  expect(moveTodo(rows, 'b', 'lane:missing')).toBeNull();
});

it('only labels the reference; completion remains a manual board decision', () => {
  expect(assignmentBadge(null)).toBeNull();
  expect(assignmentBadge({ execution_id: 'agent-1' }).label).toBe('已关联执行');
});
