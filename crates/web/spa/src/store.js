// store.js — tiny shared state on useSyncExternalStore. No redux, no OO: one
// module-level immutable snapshot plus a Set of listener callbacks (repo
// rule: pure functions, state passed/returned, no internal mutation).

import { useSyncExternalStore } from 'react';

export const TOKEN_KEY = 'oc_token';
export const BASE_KEY = 'oc_base';
/// Automatic placement; every conversation runs on its assigned node.
export const LOCAL_NODE = '__local__';
export const LOCAL_NODE_LABEL = '自动调度 / 全部会话';

let state = {
  token: localStorage.getItem(TOKEN_KEY) || '',
  // null (never stored) falls back to the build-time embedded base; an
  // explicitly stored '' still means same-origin and wins over the embed.
  base: localStorage.getItem(BASE_KEY) ?? embeddedBase(),
  page: 'nodes', // 'nodes' | 'chat' | 'dag' | 'team' | 'topics'
  preselectNode: null, // node id the fleet tab asked chat to open
  nodes: [], // last fleet snapshot shared between tabs
  conn: 'init', // 'init' | 'ok' | 'fail'
};

const listeners = new Set();

export function getState() {
  return state;
}

export function setState(patch) {
  const next = { ...state, ...patch };
  if (next === state) {
    return;
  }
  state = next;
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
  setState({ token, base: cleanBase, conn: 'init' });
}

export function clearCredentials() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(BASE_KEY);
  setState({
    token: '', base: embeddedBase(), conn: 'init', nodes: [], preselectNode: null,
  });
}

/// A 401 rejects the shared token, not the server address: clear the token
/// (plus in-flight UI state) but keep `base`, so a URL-delivered base
/// survives a bad token and the reopened login modal still points where
/// the link said. Full reset (logout) stays with clearCredentials.
export function clearToken() {
  localStorage.removeItem(TOKEN_KEY);
  setState({
    token: '', conn: 'init', nodes: [], preselectNode: null,
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

export function setNodes(nodes) {
  setState({ nodes: Array.isArray(nodes) ? nodes : [] });
}

/// Tab-1 "打开对话" → jump to tab 2 with that node preselected.
export function openChatForNode(nodeId) {
  setState({ page: 'chat', preselectNode: nodeId });
}

export function clearPreselect() {
  if (state.preselectNode !== null) {
    setState({ preselectNode: null });
  }
}
