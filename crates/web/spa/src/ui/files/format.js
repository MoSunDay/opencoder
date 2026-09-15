// Preserve numeric lexemes (including integers beyond Number.MAX_SAFE_INTEGER).
export function formatJson(text) {
  JSON.parse(text);
  const tokens = text.match(/"(?:\\.|[^"\\])*"|[{}\[\],:]|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null/g) || [];
  let depth = 0; let out = '';
  const newline = () => { out += '\n' + '  '.repeat(depth); };
  tokens.forEach((token, i) => {
    const previous = tokens[i - 1]; const next = tokens[i + 1];
    if (token === '{' || token === '[') {
      out += token; depth++;
      if (next !== '}' && next !== ']') newline();
    } else if (token === '}' || token === ']') {
      depth--;
      if (previous !== '{' && previous !== '[') newline();
      out += token;
    } else if (token === ',') { out += ','; newline(); }
    else if (token === ':') out += ': ';
    else out += token;
  });
  return out;
}

export function fileTree(paths, errors = [], changed = []) {
  const roots = []; const nodes = new Map();
  for (const path of paths) {
    let parent = roots; let key = '';
    const parts = path.split('/');
    parts.forEach((part, i) => {
      key = key ? `${key}/${part}` : part;
      if (!nodes.has(key)) {
        const leaf = i === parts.length - 1;
        const error = errors.some(e => e.path === key || !leaf && e.path.startsWith(key + '/'));
        const node = { key, title: `${error ? '⚠ ' : ''}${part}${changed.includes(key) ? ' •' : ''}`, isLeaf: leaf, ...(leaf ? {} : { children: [] }) };
        nodes.set(key, node); parent.push(node);
      }
      parent = nodes.get(key).children;
    });
  }
  return roots;
}
