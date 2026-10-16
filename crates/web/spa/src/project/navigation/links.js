const fields = { project: 'projectId', initiative: 'initiativeId', todo: 'todoId' };

export function projectLink(search, hash = '') {
  for (const value of [hash, search]) {
    const params = new URLSearchParams(value.replace(/^[?#]/, ''));
    const selection = Object.fromEntries(Object.entries(fields).map(([key, field]) => [field, params.get(key)?.trim() || null]));
    if (Object.values(selection).some(Boolean)) return selection;
  }
  return null;
}

export function projectLocation(href, selection = {}) {
  const url = new URL(href);
  const hash = new URLSearchParams(url.hash.slice(1));
  const opening = Object.values(selection).some(Boolean);
  const keys = [...Object.keys(fields), ...(opening ? ['execution', 'execution_kind', 'brain_run'] : [])];
  const updateHash = opening || keys.some((key) => hash.has(key));
  for (const key of keys) { url.searchParams.delete(key); hash.delete(key); }
  for (const [key, field] of Object.entries(fields)) {
    if (selection[field]) hash.set(key, selection[field]);
  }
  if (updateHash) url.hash = hash.toString();
  return url.pathname + url.search + url.hash;
}

export function projectTab(selection) {
  if (selection?.projectId) return 'projects';
  if (selection?.initiativeId) return 'initiatives';
  return selection?.todoId ? 'todos' : null;
}
