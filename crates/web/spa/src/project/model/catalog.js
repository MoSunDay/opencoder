import { flattenInitiatives } from './relations.js';
import { laneOf } from './board.js';

export function progressOf(todos = []) {
  const rows = [...new Map(todos.map((todo) => [todo.id, todo])).values()];
  const done = rows.filter((todo) => laneOf(todo) === 'done').length;
  return { total: rows.length, done, percent: rows.length ? Math.round(done * 100 / rows.length) : 0 };
}
export function effectiveTags(overview, initiativeId) {
  const initiative = flattenInitiatives(overview).find((item) => item.id === initiativeId);
  if (!initiative) return [];
  const names = new Map();
  for (const scope of ['project', 'initiative']) {
    for (const tag of overview?.tags || []) {
      if (tag.scope_type === scope && tag.scope_id === (scope === 'project' ? initiative.goal_id : initiative.id)) names.set(tag.name, tag);
    }
  }
  return [...names.values()].sort((a, b) => a.name.localeCompare(b.name));
}
export const todoTags = (overview, todo) => effectiveTags(overview, todo.initiative_id).filter((tag) => (todo.tag_ids || []).includes(tag.id));
export function tagGroups(rows, tags, grouped) {
  if (!grouped) return [{ id: 'all', title: '', rows }];
  return [
    ...tags.map((tag) => ({ id: tag.id, title: tag.name, rows: rows.filter((todo) => (todo.tag_ids || []).includes(tag.id)) })),
    { id: 'untagged', title: '无 Tag', rows: rows.filter((todo) => !(todo.tag_ids || []).length) },
  ];
}

export function remapTagIds(overview, ids, initiativeId) {
  const tags = effectiveTags(overview, initiativeId);
  const names = ids.map((id) => overview?.tags?.find((tag) => tag.id === id)?.name);
  return tags.filter((tag) => names.includes(tag.name)).map((tag) => tag.id);
}

export function resolveTagFilter(tags, selections) {
  return [...new Set(selections.map((selection) => (
    tags.find((tag) => tag.id === selection.id) || tags.find((tag) => tag.name === selection.name)
  )?.id).filter(Boolean))];
}
