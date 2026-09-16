// fuzzy.test.js — TUI parity contract for the subsequence scorer
// (mirrors the fuzzy_score unit tests in crates/tui/src/menu.rs).
import { describe, expect, it } from 'vitest';
import { fuzzyScore } from './fuzzy.js';

describe('fuzzyScore (TUI menu.rs parity)', () => {
  it('ranks a prefix hit best among competing targets', () => {
    const prefix = fuzzyScore('exp', 'explore');
    expect(prefix).not.toBeNull();
    // Compact consecutive match beats a scattered one (lower = better).
    const scattered = fuzzyScore('exp', 'e_x_p');
    expect(scattered).not.toBeNull();
    expect(prefix).toBeLessThan(scattered);
    // A mid-token hit loses to the same query as a prefix.
    const mid = fuzzyScore('act', 'contact');
    expect(prefix).toBeLessThan(fuzzyScore('act', 'xactx'));
  });

  it('matches sub-sequences, not sub-strings (order preserved)', () => {
    expect(fuzzyScore('epr', 'explore')).not.toBeNull();
    expect(fuzzyScore('abc', 'axbxc')).not.toBeNull();
    expect(fuzzyScore('abc', 'acb')).toBeNull();
    expect(fuzzyScore('exp', 'build')).toBeNull();
    expect(fuzzyScore('abcdef', 'abc')).toBeNull();
    expect(fuzzyScore('zz', 'act')).toBeNull();
  });

  it('returns 0 for an empty query (empty filter lists everything)', () => {
    expect(fuzzyScore('', 'anything')).toBe(0);
    expect(fuzzyScore('', '')).toBe(0);
  });

  it('is case-insensitive on both sides', () => {
    expect(fuzzyScore('CD', 'coder')).not.toBeNull();
    expect(fuzzyScore('cd', 'CODER')).not.toBeNull();
    expect(fuzzyScore('cd', 'coder')).toBe(fuzzyScore('CD', 'CODER'));
    // "cd" hits "coder" as a subsequence but not as a prefix: the compact
    // prefix-style hit must still win over a scattered one.
    expect(fuzzyScore('cd', 'coder')).toBeLessThan(fuzzyScore('cd', 'c x d'));
  });

  it('tolerates non-string input without throwing', () => {
    expect(fuzzyScore(undefined, 'x')).toBe(0);
    expect(fuzzyScore('x', null)).toBeNull();
    expect(fuzzyScore('x', 42)).toBeNull();
  });
});
