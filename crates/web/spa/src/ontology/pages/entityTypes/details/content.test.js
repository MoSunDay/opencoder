// Stylesheet contract for the entity detail text area: the HTML preview iframe
// element and the Markdown code surfaces read the dark --oc-* tokens only.
// Reads the sheet like theme.test.js does; the component side (srcDoc) is
// covered by TextPreview.test.tsx.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const sheet = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const stripComments = (source) => source.replace(/\/\*[\s\S]*?\*\//g, '');
/// Declaration block of one rule, whitespace-normalized with a trailing `;`.
const ruleBlock = (source, selector) => {
  const re = new RegExp(`(?:^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`);
  const m = re.exec(stripComments(source));
  expect(m, `${selector} rule must exist`).toBeTruthy();
  const block = m[1].replace(/\s+/g, ' ').trim();
  return block.endsWith(';') ? block : `${block};`;
};

describe('entityTypes/details/content.css', () => {
  const content = sheet('./content.css');
  const tokens = sheet('../../../../styles/tokens.css');

  it('gives the HTML preview element the same dark canvas the srcDoc body paints', () => {
    const block = ruleBlock(content, '.entity-html-preview');
    expect(block).toContain('background: var(--oc-bg-layout);');
    expect(block).toContain('border: 1px solid var(--oc-border);');
    expect(block).toContain('border-radius: 6px;');
    // Lockstep with the iframe document (TextPreview.tsx): one canvas token for
    // both surfaces, so no light page shows before or around the document.
    const preview = sheet('./TextPreview.tsx');
    expect(preview).toContain('"--oc-bg-layout": palette.bgLayout');
    expect(preview).toContain('background:var(--oc-bg-layout)');
  });

  it('keeps light-era literals out and every var(--oc-*) declared in tokens.css', () => {
    const source = stripComments(content);
    expect(source.match(/#[0-9a-f]{3,8}\b/i)).toBeNull();
    expect(source).not.toMatch(/\bwhite\b(?!\s*-)/);
    const declared = new Set([...tokens.matchAll(/(--oc-[\w-]+)\s*:/g)].map((m) => m[1]));
    for (const [, name] of source.matchAll(/var\((--oc-[\w-]+)/g)) expect(declared.has(name), name).toBe(true);
  });

  it('styles the Markdown code surfaces with the same tokens as the iframe document', () => {
    const pre = ruleBlock(content, '.entity-text-preview pre');
    expect(pre).toContain('background: var(--oc-fill-subtle);');
    expect(pre).toContain('border: 1px solid var(--oc-border);');
    expect(pre).toContain('font-family: var(--oc-mono);');
    expect(ruleBlock(content, '.entity-text-preview code')).toContain('background: var(--oc-fill-subtle);');
  });
});
