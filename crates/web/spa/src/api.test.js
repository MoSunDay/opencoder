// api.test.js — P0-1 regression: authFetch must route through urlFor so a
// cross-origin deployment (server base entered on the login screen) reaches
// the configured origin, not the page origin. The original bug imported
// urlFor but fetched `pathAndQuery` directly.
// @vitest-environment jsdom
// (store.js touches localStorage at import time; jsdom provides it.)
import { afterEach, describe, expect, it, vi } from 'vitest';
import { apiPost, authFetch } from './api.js';
import { clearCredentials, getState, setCredentials } from './store.js';

const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({}) }));
vi.stubGlobal('fetch', fetchMock);

afterEach(() => {
  fetchMock.mockClear();
  vi.restoreAllMocks();
  clearCredentials();
});

describe('authFetch base routing', () => {
  it('prefixes the configured server base onto the fetched URL', async () => {
    setCredentials('tok', 'http://10.0.0.9:8080');
    await authFetch('POST', '/api/sessions/s1/prompt', { prompt: 'hi' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://10.0.0.9:8080/api/sessions/s1/prompt');
    expect(init.headers.Authorization).toBe('Bearer tok');
    expect(init.headers['x-sig']).toBeUndefined();
  });

  it('stays same-origin when no base is configured', async () => {
    setCredentials('tok', '');
    await authFetch('GET', '/api/sessions?limit=50');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe('/api/sessions?limit=50');
  });

  it('clears rejected credentials after one mutation 401 without retrying', async () => {
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 401,
      json: async () => ({ error: 'invalid bearer token' }),
    });
    setCredentials('wrong', '');
    await expect(apiPost('/api/executions', { kind: 'agent' })).rejects.toMatchObject({ status: 401 });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(getState().token).toBe('');
    expect(localStorage.getItem('oc_token')).toBeNull();
  });
});
