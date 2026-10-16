// api.test.js — P0-1 regression: authFetch must route through urlFor so a
// cross-origin deployment (server base entered on the login screen) reaches
// the configured origin, not the page origin. The original bug imported
// urlFor but fetched `pathAndQuery` directly.
// @vitest-environment jsdom
// (store.js touches localStorage at import time; jsdom provides it.)
import { afterEach, describe, expect, it, vi } from 'vitest';
import { apiPost, authFetch } from './api.js';
import { clearCredentials, getState, setCredentials, setState } from './store.js';

const fetchMock = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({}) }));
vi.stubGlobal('fetch', fetchMock);

afterEach(() => {
  fetchMock.mockClear();
  vi.restoreAllMocks();
  clearCredentials();
});

describe('authFetch base routing', () => {
  it('sends raw file bytes with the same bearer and base URL', async () => {
    setCredentials('test-token', 'http://localhost:8080'); setState({ authEnabled: true });
    const file = new Blob(['raw bytes']);
    await authFetch('PUT', '/api/project/todos/t/file?path=a', undefined, { rawBody: file, contentType: 'application/octet-stream' });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://localhost:8080/api/project/todos/t/file?path=a');
    expect(init.body).toBe(file);
    expect(init.headers).toEqual({ Authorization: 'Bearer test-token', 'content-type': 'application/octet-stream' });
  });
  it('omits Authorization when the server has disabled authentication', async () => {
    setCredentials('previous-token', '');
    setState({ authEnabled: false });
    await apiPost('/api/project/goals', { title: 'project' });
    expect(fetchMock.mock.calls[0][1].headers.Authorization).toBeUndefined();
    expect(getState().token).toBe('previous-token');
  });
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

  it('keeps the server base when a 401 rejects the token', async () => {
    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 401,
      json: async () => ({ error: 'invalid bearer token' }),
    });
    setCredentials('wrong', 'http://fleet2.example.com');
    await expect(apiPost('/api/executions', { kind: 'agent' })).rejects.toMatchObject({ status: 401 });
    expect(getState().token).toBe('');
    // The token was rejected, not the address: base survives the 401 so the
    // reopened login modal (and a #base= link) still points where it should.
    expect(getState().base).toBe('http://fleet2.example.com');
    expect(localStorage.getItem('oc_base')).toBe('http://fleet2.example.com');
  });
});
