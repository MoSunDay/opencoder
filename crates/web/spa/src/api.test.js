// api.test.js — P0-1 regression: signFetch must route through urlFor so a
// cross-origin deployment (server base entered on the login screen) reaches
// the configured origin, not the page origin. The original bug imported
// urlFor but fetched `pathAndQuery` directly — every signed call after a
// cross-origin login 404'd against the page origin (only /api/time worked,
// being base-prefixed in time.js).
// @vitest-environment jsdom
// (store.js touches localStorage at import time; jsdom provides it.)
import { afterEach, describe, expect, it, vi } from 'vitest';
import { signFetch } from './api.js';
import { setCredentials } from './store.js';

const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({}) }));
vi.stubGlobal('fetch', fetchMock);

afterEach(() => {
  fetchMock.mockClear();
  setCredentials('', '');
});

describe('signFetch base routing', () => {
  it('prefixes the configured server base onto the fetched URL', async () => {
    setCredentials('tok', 'http://10.0.0.9:8080');
    await signFetch('POST', '/api/sessions/s1/prompt', { prompt: 'hi' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe('http://10.0.0.9:8080/api/sessions/s1/prompt');
  });

  it('stays same-origin when no base is configured', async () => {
    setCredentials('tok', '');
    await signFetch('GET', '/api/sessions?limit=50');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe('/api/sessions?limit=50');
  });
});
