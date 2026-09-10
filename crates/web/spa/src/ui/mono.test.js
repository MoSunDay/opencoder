// mono.test.js — pure-node guard for the monospace single-source rule. Three
// surfaces must agree on ONE stack, and this test reads the real other half
// (app.css on disk) rather than re-reading theme.js — because theme.js
// literally sets `'--oc-mono': MONO`, comparing MONO to cssVars['--oc-mono']
// is a tautology (MONO === MONO) that guards nothing. What is actually pinned:
//   1. MONO (the JS constant) <-> the --oc-mono declaration in app.css :root
//      (raw CSS surfaces like .md-body code consume the property directly);
//   2. MONO <-> theme.token.fontFamilyCode (antd's <Typography.Text code>
//      face, which otherwise falls back to a Courier-bearing default);
//   3. MONO_VAR resolves through the property with a bare generic fallback.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { MONO, MONO_VAR } from './mono.js';
import { theme } from '../theme.js';

const css = readFileSync(new URL('../app.css', import.meta.url), 'utf8');

/// Comments stripped first: the :root block carries explanatory comments
/// between declarations, and a value must never absorb one (mirrors the
/// helpers in theme.test.js — deliberately not imported from there).
const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');

/// Prettier wraps the font list across lines, so compare whitespace-normalized.
const squash = (v) => String(v).replace(/\s+/g, ' ').trim().toLowerCase();

/// The raw `--oc-mono: …;` value out of the app.css :root block.
const declaredMono = () => {
  const block = /:root\s*\{([\s\S]*?)\n\}/.exec(stripComments(css));
  expect(block, 'app.css must declare a :root block').toBeTruthy();
  const m = /--oc-mono\s*:\s*([^;]+);/.exec(block[1]);
  expect(m, 'app.css :root must declare --oc-mono').toBeTruthy();
  return m[1];
};

describe('mono stack', () => {
  it('is the same stack the --oc-mono custom property declares in app.css', () => {
    // Two copies of a font stack is how panels end up in different faces.
    expect(squash(MONO)).toBe(squash(declaredMono()));
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
});
