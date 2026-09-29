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

it('only calls a TODO complete when the latest assignment has a result', () => {
  expect(assignmentBadge({ has_result: true }).label).toBe('结论已回写');
  expect(assignmentBadge({ has_result: false, sync_state: 'error' }).label).toBe('执行失败');
  expect(assignmentBadge({ has_result: false, sync_state: 'cancelled' }).label).toBe('已取消');
  expect(assignmentBadge({ has_result: false, sync_state: 'empty' }).label).toBe('已结束，无结论');
  expect(assignmentBadge({ has_result: false, sync_state: 'pending' }).label).toBe('已指派，待结论');
});
