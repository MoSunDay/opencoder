// Check the font stack against actual CSS, antd tokens and MONO_VAR.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { MONO, MONO_VAR } from './mono.js';
import { theme } from '../theme.js';

/// --oc-mono lives in the token sheet (app.css imports it first).
const css = readFileSync(new URL('../styles/tokens.css', import.meta.url), 'utf8');

/// Comments stripped first: the :root block carries explanatory comments
/// between declarations, and a value must never absorb one (mirrors the
/// helpers in theme.test.js - deliberately not imported from there).
const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');

/// Prettier wraps the font list across lines, so compare whitespace-normalized.
const squash = (v) => String(v).replace(/\s+/g, ' ').trim().toLowerCase();

/// The raw `--oc-mono: ...;` value out of the token stylesheet :root block.
const declaredMono = () => {
  const block = /:root\s*\{([\s\S]*?)\n\}/.exec(stripComments(css));
  expect(block, 'tokens.css must declare a :root block').toBeTruthy();
  const m = /--oc-mono\s*:\s*([^;]+);/.exec(block[1]);
  expect(m, 'tokens.css :root must declare --oc-mono').toBeTruthy();
  return m[1];
};

/// Ordered face list of a stack (whitespace-normalized, split on commas).
const faces = (stack) => squash(stack).split(',').map((f) => f.trim());

describe('mono stack', () => {
  it('is the same stack the --oc-mono custom property declares in the token sheet', () => {
    // Two copies of a font stack is how panels end up in different faces.
    expect(squash(MONO)).toBe(squash(declaredMono()));
    expect(faces(MONO)).toEqual(faces(declaredMono()));
  });

  it('pins the antd code token to the same stack', () => {
    // Without fontFamilyCode antd uses its own default code face (Courier, no
    // ui-monospace), so inline code would fork from every MONO_VAR identifier.
    expect(theme.token.fontFamilyCode).toBe(MONO);
  });

  it('resolves through the custom property with a bare generic fallback', () => {
    expect(MONO_VAR).toBe('var(--oc-mono, monospace)');
  });

  it('covers the platforms the console actually runs on', () => {
    for (const face of ['ui-monospace', 'SFMono-Regular', "'SF Mono'", 'Menlo', 'Consolas', "'Liberation Mono'", 'monospace']) {
      expect(MONO, `missing ${face}`).toContain(face);
    }
  });

  it('lists the Latin monospace faces first and the Chinese fallbacks before the generic', () => {
    // Monospace fonts carry no CJK glyphs: the CJK faces must follow every
    // Latin mono face (so Latin text stays monospace) and precede the bare
    // generic, otherwise a Chinese identifier falls to the browser default.
    const list = faces(MONO);
    const at = (face) => {
      const i = list.indexOf(face.toLowerCase());
      expect(i, `missing ${face}`).toBeGreaterThan(-1);
      return i;
    };
    expect(list).toHaveLength(10);
    expect(at("'Liberation Mono'")).toBeLessThan(at("'Noto Sans Mono CJK SC'"));
    expect(at("'Noto Sans Mono CJK SC'")).toBeLessThan(at("'PingFang SC'"));
    expect(at("'PingFang SC'")).toBeLessThan(at("'Microsoft YaHei'"));
    expect(at("'Microsoft YaHei'")).toBe(list.length - 2);
    expect(list[list.length - 1]).toBe('monospace');
  });
});
