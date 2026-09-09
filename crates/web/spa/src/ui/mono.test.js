// mono.test.js — pure-node guard for the monospace single-source rule.

import { describe, expect, it } from 'vitest';
import { MONO, MONO_VAR } from './mono.js';
import { cssVars } from '../theme.js';

const squash = (v) => String(v).replace(/\s+/g, ' ').trim().toLowerCase();

describe('mono stack', () => {
  it('is the same stack the --oc-mono custom property declares', () => {
    // Two copies of a font stack is how panels end up in different faces.
    expect(squash(MONO)).toBe(squash(cssVars['--oc-mono']));
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
