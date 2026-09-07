// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { markdownRows, rowText, sayPresentation } from './markdown.js';
const say = (text) => [{ kind: 'text', role: 'assistant', text }];
const texts = (rows) => rows.map(rowText);

describe('TUI Say preview and body use the same rendered lines', () => {
  it('renders heading and inline markup once, retaining styled body text', () => {
    const p = sayPresentation(say('# heading\n**body** and *detail*'));
    expect(p.preview).toBe('heading');
    expect(texts(p.rows)).toEqual(['', 'body and detail']);
    expect(p.rows.flat().find((part) => part.text === 'body').bold).toBe(true);
    expect(p.rows.flat().find((part) => part.text === 'detail').italic).toBe(true);
  });
  it('keeps streaming raw then switches to rendered preview without duplicates', () => {
    const parts = say('\n# heading\nbody');
    expect(sayPresentation(parts, true).preview).toBe('# heading');
    expect(texts(sayPresentation(parts, true).rows)).toEqual(['body']);
    expect(sayPresentation(parts).preview).toBe('heading');
    expect(sayPresentation(say('**single**')).rows).toEqual([]);
    expect(sayPresentation(say('\n\n  '))).toEqual({ preview: '', rows: [], other: [] });
  });
  it('retains code lines and closing fence when the opening label is the preview', () => {
    const p = sayPresentation(say('```rust\nfn main() {}\n\n// tail\n```'));
    expect(p.preview).toBe('┌ rust');
    expect(texts(p.rows)).toEqual(['│ fn main() {}', '│', '│ // tail', '└' + '─'.repeat(19)]);
  });
  it('preserves list numbering, nested items, entities, links and inline code', () => {
    const p = sayPresentation(say('1. **first** &amp; one\n2. `second`\n   - [child](https://example.com)'));
    expect(p.preview).toBe('1. first & one');
    expect(texts(p.rows)).toEqual(['2. `second`', '  • [child]']);
    expect(texts(markdownRows('> quote\n\n---'))).toEqual(['▎ quote', '', '─'.repeat(19)]);
  });
  it('never treats an image marker as Say text or executes authored HTML', () => {
    const image = { kind: 'text', role: 'assistant', text: '[image]', image: true };
    expect(sayPresentation([image])).toEqual({ preview: '', rows: [], other: [image] });
    expect(texts(markdownRows('<script>alert(1)</script>\n\n**safe**'))).toEqual(['safe']);
    expect(document.querySelector('script')).toBeNull();
  });
});
