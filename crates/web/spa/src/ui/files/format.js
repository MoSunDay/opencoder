import {jsonLanguage} from '@codemirror/lang-json';

export function jsonLocation(text) {
  let offset=text.length;
  jsonLanguage.parser.parse(text).iterate({enter(node){if(node.type.isError)offset=Math.min(offset,node.from);}});
  const prefix=text.slice(0,offset);
  return {line:prefix.split('\n').length,column:prefix.length-prefix.lastIndexOf('\n')};
}

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

export function directoryPaths(files, directories = []) {
  return [...new Set([...directories, ...Object.keys(files).flatMap(path =>
    path.split('/').slice(0, -1).map((_, index) => path.split('/').slice(0, index + 1).join('/')))])].sort();
}

export function fileTree(paths, errors = [], changed = [], directories = []) {
  const roots = []; const nodes = new Map();
  const folders = new Set(directoryPaths(Object.fromEntries(paths.map(path => [path, ''])), directories));
  for (const path of [...new Set([...folders, ...paths])].sort()) {
    let parent = roots; let key = '';
    const parts = path.split('/');
    parts.forEach((part, i) => {
      key = key ? `${key}/${part}` : part;
      if (!nodes.has(key)) {
        const leaf = i === parts.length - 1 && !folders.has(key);
        const error = errors.some(e => e.path === key || !leaf && e.path.startsWith(key + '/'));
        const node = { key, title: `${error ? '⚠ ' : ''}${part}${changed.includes(key) ? ' •' : ''}`, isLeaf: leaf, ...(leaf ? {} : { children: [] }) };
        nodes.set(key, node); parent.push(node);
      }
      parent = nodes.get(key).children;
    });
  }
  const sort = entries => {
    entries.sort((a, b) => Number(a.isLeaf) - Number(b.isLeaf) || a.key.localeCompare(b.key));
    entries.forEach(node => {if (node.children) sort(node.children);});
  };
  sort(roots);
  return roots;
}
