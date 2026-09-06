// Pure-node unit tests (T1) for urlCredential.js — no jsdom, real URL parsing.

import { describe, expect, it } from 'vitest';
import { urlCredential } from './urlCredential.js';

describe('urlCredential', () => {
  it('captures a query token and scrubs only it', () => {
    const out = urlCredential('http://h:1/?view=nodes&token=url-secret&x=2#fleet');
    expect(out.captured).toBe(true);
    expect(out.token).toBe('url-secret');
    expect(out.clean).toBe('/?view=nodes&x=2#fleet');
  });

  it('captures a fragment token (preferred channel)', () => {
    const out = urlCredential('http://h:1/#token=frag-secret');
    expect(out.token).toBe('frag-secret');
    expect(out.clean).toBe('/');
  });

  it('captures base alongside token from either channel', () => {
    const out = urlCredential('http://h:1/#base=https://fleet.example.com&token=t1');
    expect(out).toMatchObject({ token: 't1', base: 'https://fleet.example.com' });
    expect(out.clean).toBe('/');
  });

  it('preserves unrelated fragment params while scrubbing credentials', () => {
    const out = urlCredential('http://h:1/page?keep=1#token=t&tab=dag');
    expect(out.clean).toBe('/page?keep=1#tab=dag');
  });

  it('leaves anchor-style hashes and credential-free URLs untouched', () => {
    expect(urlCredential('http://h:1/?view=nodes#fleet')).toEqual({
      captured: false, token: '', base: '', clean: null,
    });
  });

  it('drops a query token that only had whitespace', () => {
    const out = urlCredential('http://h:1/?token=%20%20');
    expect(out.captured).toBe(false);
  });
});
