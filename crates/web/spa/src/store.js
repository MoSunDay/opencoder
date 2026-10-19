// store.js — tiny shared state on useSyncExternalStore. No redux, no OO: one
// module-level immutable snapshot plus a Set of listener callbacks (repo
// rule: pure functions, state passed/returned, no internal mutation).

import { useSyncExternalStore } from 'react';
import { linkedPage } from './fleet/navigation/links.js';

export const TOKEN_KEY = 'oc_token';
export const BASE_KEY = 'oc_base';

let state = {
  token: localStorage.getItem(TOKEN_KEY) || '',
  // null (never stored) falls back to the build-time embedded base; an
  // explicitly stored '' still means same-origin and wins over the embed.
  base: localStorage.getItem(BASE_KEY) ?? embeddedBase(),
  page: linkedPage(window.location.search, window.location.hash) || 'nodes', // 'nodes' | 'chat' | 'dag' | 'team' | 'topics'
  nodes: [], // last fleet snapshot shared between tabs
  conn: 'init', // 'init' | 'ok' | 'fail'
  identity: null, // {name, role} from GET /api/me; null until probed
  authEnabled: null, // Public /api/auth/status; never persisted.
};

const listeners = new Set();

export function getState() {
  return state;
}

export function setState(patch) {
  state = { ...state, ...patch };
  listeners.forEach((fn) => fn());
}

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function useStore() {
  return useSyncExternalStore(subscribe, getState);
}

/// Build-time embedded server base (VITE_OC_BASE at `vite build` time) —
/// baked into the bundle so a standalone SPA ships pre-pointed at its fleet.
/// Read at call time (not import time) to stay unit-testable.
export function embeddedBase() {
  return String(import.meta.env.VITE_OC_BASE || '').trim().replace(/\/+$/, '');
}

/// Persist + publish credentials. `base` is stored exactly as typed ('' =
/// same-origin).
export function setCredentials(token, base) {
  const cleanBase = String(base || '').trim().replace(/\/+$/, '');
  localStorage.setItem(TOKEN_KEY, token);
  localStorage.setItem(BASE_KEY, cleanBase);
  // identity is re-probed after login (GET /api/me in login.jsx).
  setState({ token, base: cleanBase, conn: 'init', identity: null, authEnabled: null });
}

/// Publish the authenticated identity ({name, role}) resolved by the login
/// probe (GET /api/me). Merges into state; panels/nav read it via useStore.
export function setIdentity(identity) {
  setState({ identity: identity && identity.name ? { name: identity.name, role: identity.role || 'viewer' } : null });
}

export function clearCredentials() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(BASE_KEY);
  setState({
    token: '', base: embeddedBase(), conn: 'init', nodes: [], identity: null, authEnabled: null,
  });
}

/// A 401 rejects the shared token, not the server address: clear the token
/// (plus in-flight UI state) but keep `base`, so a URL-delivered base
/// survives a bad token and the reopened login modal still points where
/// the link said. Full reset (logout) stays with clearCredentials.
export function clearToken() {
  localStorage.removeItem(TOKEN_KEY);
  setState({
    token: '', conn: 'init', nodes: [], identity: null, authEnabled: true,
  });
}

/// Origin-prefixing helper shared by api.js and sse.js.
export function urlFor(pathAndQuery) {
  return (state.base || '') + pathAndQuery;
}

export function setConn(conn) {
  if (state.conn !== conn) {
    setState({ conn });
  }
}
