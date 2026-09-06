// api.js — Bearer-authenticated fetch plumbing shared by JSON and SSE callers.

import { clearToken, getState, setConn, urlFor } from './store.js';

function token() {
  return getState().token;
}

/// authFetch(method, pathAndQuery, bodyObj) — bodyObj undefined for GET/DELETE.
/// Returns the raw Response (streaming callers need response.body).
export async function authFetch(method, pathAndQuery, bodyObj, opts = {}) {
  const m = String(method).toUpperCase();
  const bodyText = bodyObj === undefined ? '' : JSON.stringify(bodyObj);
  const headers = { Authorization: 'Bearer ' + token() };
  if (bodyText) {
    headers['content-type'] = 'application/json';
  }
  const response = await fetch(urlFor(pathAndQuery), {
    method: m,
    headers,
    body: bodyText || undefined,
    signal: opts.signal,
  });
  if (response.status === 401) {
    // Only the token was rejected — keep the server base so the reopened
    // login modal (and a URL-delivered base) survives the 401.
    clearToken();
  }
  return response;
}

function noteConn(ok) {
  setConn(ok ? 'ok' : 'fail');
}

/// JSON convenience: throws Error({status, message}) on non-2xx, returns the
/// parsed body. Errors carry the server's `error` field when present.
export async function apiJson(method, pathAndQuery, bodyObj, opts = {}) {
  let resp;
  try {
    resp = await authFetch(method, pathAndQuery, bodyObj, opts);
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
