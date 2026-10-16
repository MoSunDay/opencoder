import { marked } from 'marked';

function readable(tokens, separator = ' ') {
  return tokens.map((token) => {
    if (token.type === 'html') return '';
    if (token.items) return token.items.map((item) => readable(item.tokens || [])).join(' ');
    if (token.header) return [...token.header, ...token.rows.flat()].map((cell) => readable(cell.tokens || [])).join(' ');
    if (token.tokens) return readable(token.tokens, '');
    return token.text || '';
  }).join(separator);
}

export function taskSummary(markdown) {
  if (!markdown) return '';
  const text = readable(marked.lexer(String(markdown))).replace(/\s+/g, ' ').trim();
  return text.length > 240 ? `${text.slice(0, 240)}…` : text;
}
