// Markdown → styled logical lines, matching tui/src/markdown.rs. Keeping
// preview and body in one representation avoids stripping a raw Markdown
// line (which corrupts fences, lists and multiline inline formatting).
import { marked } from 'marked';
import { sayBodyParts, sayPreview } from '../sayText.js';

const RULE = '─'.repeat(19);
const span = (text, style = {}) => ({ text, ...style });
export const rowText = (row) => row.map((part) => part.text).join('');

function decoded(text) {
  // Detached text decoder: never insert authored HTML into the document.
  const element = document.createElement('textarea');
  element.innerHTML = String(text).replace(/</g, '&lt;');
  return element.value;
}

function inline(tokens, style = {}) {
  return (tokens || []).flatMap((token) => {
    switch (token.type) {
      case 'strong': return inline(token.tokens, { ...style, bold: true });
      case 'em': return inline(token.tokens, { ...style, italic: true });
      case 'del': return inline(token.tokens, { ...style, strike: true });
      case 'codespan': return [span('`' + decoded(token.text) + '`', { ...style, code: true })];
      case 'link': return [span('[', style), ...inline(token.tokens, { ...style, link: true }), span(']', style)];
      case 'image': return [span(decoded(token.text || ''), style)];
      case 'br': return [span('\n', style)];
      case 'html': return [];
      default: return token.tokens ? inline(token.tokens, style) : [span(decoded(token.text || ''), style)];
    }
  });
}

function lines(spans) {
  const rows = [[]];
  for (const part of spans) {
    part.text.split('\n').forEach((text, i) => {
      if (i) rows.push([]);
      if (text) rows.at(-1).push({ ...part, text });
    });
  }
  return rows;
}

function blocks(tokens, depth = 0, style = {}) {
  return (tokens || []).flatMap((token) => {
    switch (token.type) {
      case 'heading': return [...lines(inline(token.tokens, { ...style, heading: token.depth, bold: true })), []];
      case 'paragraph': return [...lines(inline(token.tokens, style)), []];
      case 'text': return lines(inline(token.tokens || [token], style));
      case 'space': case 'def': case 'html': return [];
      case 'hr': return [[span(RULE, { muted: true })]];
      case 'code': {
        const code = token.text.replace(/\n$/, '').split('\n');
        return [
          [span('┌ ' + (token.lang || '') + ' ', { muted: true })],
          ...code.map((text) => [span(text ? '│ ' : '│', { muted: true }), span(text, { code: true })]),
          [span('└' + RULE, { muted: true })], [],
        ];
      }
      case 'blockquote': {
        const rows = blocks(token.tokens, depth, { ...style, muted: true });
        if (rows.length) rows[0] = [span('▎ ', { muted: true }), ...rows[0]];
        return rows;
      }
      case 'list': return token.items.flatMap((item, index) => {
        const rows = blocks(item.tokens, depth + 1, style);
        const prefix = '  '.repeat(depth) + (token.ordered ? `${index + 1}. ` : '• ');
        if (!rows.length) rows.push([]);
        rows[0] = [span(prefix, style), ...rows[0]];
        return rows;
      });
      case 'table': {
        // TUI's table events retain all cell text in row order. On the Web
        // use explicit column separators so columns remain distinguishable.
        const tableRow = (cells) => cells.flatMap((cell, i) => [
          ...(i ? [span(' │ ', { muted: true })] : []), ...inline(cell.tokens, style),
        ]);
        return [tableRow(token.header), ...token.rows.map(tableRow), []];
      }
      default: return token.tokens ? blocks(token.tokens, depth, style) : [];
    }
  });
}

export function markdownRows(text) {
  const rows = blocks(marked.lexer(String(text || ''), { gfm: true }));
  while (rows.length && !rowText(rows.at(-1)).trim()) rows.pop();
  return rows;
}

export function textRows(text, streaming = false) {
  return streaming ? String(text || '').split('\n').map((line) => [span(line)]) : markdownRows(text);
}

export function sayPresentation(parts, streaming = false) {
  const say = Array.isArray(parts) ? parts : [];
  const isText = (part) => part?.kind === 'text' && typeof part.text === 'string' && !part.image;
  if (streaming) {
    const preview = sayPreview(say);
    const body = sayBodyParts(say, preview);
    const rows = textRows(body.filter(isText).map((part) => part.text).join(''), true);
    while (rows.length && !rowText(rows[0]).trim()) rows.shift();
    return { preview, rows, other: body.filter((part) => !isText(part)) };
  }
  const rows = textRows(say.filter(isText).map((part) => part.text).join(''), streaming);
  const first = rows.findIndex((row) => rowText(row).trim());
  const body = first < 0 ? [] : rows.slice(first + 1);
  return {
    preview: first < 0 ? '' : rowText(rows[first]).trim(),
    rows: body.some((row) => rowText(row).trim()) ? body : [],
    other: say.filter((part) => !isText(part)),
  };
}
