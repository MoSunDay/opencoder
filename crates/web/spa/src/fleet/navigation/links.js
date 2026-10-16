export function executionHref(id, kind) {
  const query = new URLSearchParams({ execution: id });
  if (kind) query.set('execution_kind', kind);
  // Fragments survive redirects that replace the page's query string.
  return `#${query}`;
}

export function executionLink(search, hash = '') {
  for (const value of [hash, search]) {
    const query = new URLSearchParams(value.replace(/^[?#]/, ''));
    const id = query.get('execution')?.trim();
    if (id) return { id, kind: query.get('execution_kind') || '' };
  }
  return null;
}

export function linkedPage(search, hash = '') {
  if (executionLink(search, hash)) return 'topics';
  return new URLSearchParams(search).has('brain_run') ? 'brain' : null;
}

export function withoutExecutionLink(href) {
  const url = new URL(href);
  url.searchParams.delete('execution'); url.searchParams.delete('execution_kind');
  const fragment = new URLSearchParams(url.hash.slice(1));
  if (fragment.has('execution') || fragment.has('execution_kind')) {
    fragment.delete('execution'); fragment.delete('execution_kind');
    url.hash = fragment.toString();
  }
  return url.pathname + url.search + url.hash;
}
