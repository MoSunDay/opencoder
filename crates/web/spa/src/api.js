// api.js — signed fetch plumbing. Every /api call except GET /api/time goes
// through signFetch: canonical string over method+path+query+body hash, signed
// with the shared token (mirror of crates/core/src/auth_sig.rs). On 401 the
// clock offset is re-derived from /api/time once and the request is retried
// once — a drifted browser clock is by far the most common failure.

import { signRequest } from './sign.js';
import { clockOffsetMs, syncTime } from './time.js';
import { getState, setConn, urlFor } from './store.js';

function token() {
  return getState().token;
}

/// signFetch(method, pathAndQuery, bodyObj) — bodyObj undefined for GET/DELETE.
/// Returns the raw Response (streaming callers need response.body).
export async function signFetch(method, pathAndQuery, bodyObj, opts = {}) {
  const m = String(method).toUpperCase();
  const bodyText = bodyObj === undefined ? '' : JSON.stringify(bodyObj);
  const doAttempt = async () => {
    const { ts, sig } = await signRequest(token(), m, pathAndQuery, new TextEncoder().encode(bodyText), Date.now() + clockOffsetMs());
    const headers = { 'x-sig-timestamp': String(ts), 'x-sig': sig };
    if (bodyText) {
      headers['content-type'] = 'application/json';
    }
    // `urlFor` prefixes the configured server base ('' = same-origin). Without
    // it every signed request after a cross-origin login hit THIS origin and
    // 404'd (only /api/time worked — it is signature-exempt and already
    // base-prefixed in time.js).
    return fetch(urlFor(pathAndQuery), {
      method: m,
      headers,
      body: bodyText || undefined,
      signal: opts.signal,
    });
  };
  let resp = await doAttempt();
  if (resp.status === 401 && !opts.retried) {
    await syncTime().catch(() => {});
    resp = await doAttempt();
  }
  return resp;
}

function noteConn(ok) {
  setConn(ok ? 'ok' : 'fail');
}

/// JSON convenience: throws Error({status, message}) on non-2xx, returns the
/// parsed body. Errors carry the server's `error` field when present.
export async function apiJson(method, pathAndQuery, bodyObj, opts = {}) {
  let resp;
  try {
    resp = await signFetch(method, pathAndQuery, bodyObj, opts);
  } catch (e) {
    if (e && e.name === 'AbortError') {
      throw e;
    }
    noteConn(false);
    throw Object.assign(new Error('网络错误: ' + (e && e.message)), { status: 0 });
  }
  let body = null;
  try {
    body = await resp.json();
  } catch {
    body = null; // 204/empty bodies are legal
  }
  if (!resp.ok) {
    noteConn(resp.status !== 401);
    const msg = (body && body.error) || 'HTTP ' + resp.status;
    throw Object.assign(new Error(msg), { status: resp.status, body });
  }
  noteConn(true);
  return body;
}

export const apiGet = (path, opts) => apiJson('GET', path, undefined, opts);
export const apiPost = (path, body, opts) => apiJson('POST', path, body === undefined ? {} : body, opts);
export const apiPut = (path, body, opts) => apiJson('PUT', path, body === undefined ? {} : body, opts);
export const apiPatch = (path, body, opts) => apiJson('PATCH', path, body === undefined ? {} : body, opts);
export const apiDel = (path, opts) => apiJson('DELETE', path, undefined, opts);
