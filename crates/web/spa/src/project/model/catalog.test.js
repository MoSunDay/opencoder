import { expect, it } from 'vitest';
import { effectiveTags, progressOf, remapTagIds, resolveTagFilter, tagGroups } from './catalog.js';
import { moveTodo } from './board.js';
const overview = { goals: [{ id: 'p', initiatives: [{ id: 'i', todos: [] }] }], tags: [
  { id: 'p1', scope_type: 'project', scope_id: 'p', name: '模块' }, { id: 'p2', scope_type: 'project', scope_id: 'p', name: '重点' },
  { id: 'i1', scope_type: 'initiative', scope_id: 'i', name: '模块' }, { id: 'x', scope_type: 'initiative', scope_id: 'other', name: '其他' },
] };
it('resolves same-name initiative tags while preserving other project tags', () => {
  expect(effectiveTags(overview, 'i').map((t) => t.id).sort()).toEqual(['i1', 'p2']);
  expect(effectiveTags(overview, null)).toEqual([]);
  expect(remapTagIds(overview, ['p1', 'p2', 'x'], 'i').sort()).toEqual(['i1', 'p2']);
});
it('groups multi-tag TODOs without duplicating progress and retains untagged work', () => {
  const rows = [{ id: 'a', board_status: 'done', tag_ids: ['i1', 'p2'] }, { id: 'b', board_status: 'todo', tag_ids: [] }];
  const groups = tagGroups(rows, effectiveTags(overview, 'i'), true);
  expect(groups.map((g) => g.rows.map((t) => t.id))).toEqual([['a'], ['a'], ['b']]);
  expect(progressOf(groups.flatMap((g) => g.rows))).toEqual({ total: 2, done: 1, percent: 50 });
  expect(progressOf([])).toEqual({ total: 0, done: 0, percent: 0 });
});
it('preserves hidden cards when dragging onto a visible card or empty lane', () => {
  const rows = [{ id: 'a', board_status: 'todo', position: 1000, tag_ids: ['i1'] }, { id: 'hidden', board_status: 'done', position: 1000 }, { id: 'b', board_status: 'done', position: 2000 }];
  const moved = moveTodo(rows, 'a', 'b');
  expect(moved.ids).toEqual(['hidden', 'a', 'b']);
  expect(moved.preview[0].tag_ids).toEqual(['i1']);
  expect(moveTodo(rows, 'a', 'lane:in_progress').ids).toEqual(['a']);
});
it('weights project completion by the actual TODO count', () => {
  const rows = [{ id: 'a', board_status: 'done' }, ...Array.from({ length: 9 }, (_, i) => ({ id: `b${i}`, board_status: i ? 'todo' : 'done' }))];
  expect(progressOf(rows).percent).toBe(20);
});
it('retains tag filters on rename and same-name scope changes', () => {
  const tags = [{ id: 'local', name: '模块' }, { id: 'focus', name: '已改名' }];
  expect(resolveTagFilter(tags, [{ id: 'parent', name: '模块' }, { id: 'focus', name: '重点' }])).toEqual(['local', 'focus']);
  expect(resolveTagFilter(tags, [{ id: 'deleted', name: '已删除' }])).toEqual([]);
});
