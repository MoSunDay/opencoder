// Move in the complete list, so filtering never drops or reverses hidden rows.
export function moveInitiative(rows, activeId, overId) {
  const from = rows.findIndex((row) => row.id === activeId);
  const to = rows.findIndex((row) => row.id === overId);
  if (from < 0 || to < 0 || from === to) return null;
  const next = [...rows];
  next.splice(to, 0, next.splice(from, 1)[0]);
  return next;
}
