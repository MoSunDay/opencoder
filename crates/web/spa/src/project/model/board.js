export const LANES = [
  ['backlog', '待整理'], ['todo', '待办'], ['in_progress', '进行中'], ['done', '已完成'],
];

export const laneOf = (todo) => todo.board_status || ({ draft: 'backlog', planned: 'todo', running: 'in_progress', done: 'done' }[todo.status] || 'todo');

export function assignmentBadge(assignment) {
  if (!assignment) return null;
  return { color: 'blue', label: '已关联执行' };
}

export const ordered = (rows) => [...rows].sort((a, b) => (a.position ?? a.created_at ?? 0) - (b.position ?? b.created_at ?? 0) || a.id.localeCompare(b.id));

export function moveTodo(all, activeId, overId) {
  const moved = all.find((todo) => todo.id === activeId);
  const target = all.find((todo) => todo.id === overId);
  const status = target ? laneOf(target) : String(overId).replace(/^lane:/, '');
  if (!moved || !LANES.some(([id]) => id === status)) return null;
  const rows = ordered(all.filter((todo) => laneOf(todo) === status && todo.id !== moved.id));
  const at = target ? rows.findIndex((todo) => todo.id === target.id) : rows.length;
  rows.splice(at < 0 ? rows.length : at, 0, moved);
  const positions = new Map(rows.map((todo, index) => [todo.id, (index + 1) * 1000]));
  return {
    status,
    ids: rows.map((todo) => todo.id),
    preview: all.map((todo) => positions.has(todo.id) ? { ...todo, board_status: status, position: positions.get(todo.id) } : todo),
  };
}
