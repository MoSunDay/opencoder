// store.js — tiny shared state on useSyncExternalStore. No redux, no OO: one
// module-level immutable snapshot plus a Set of listener callbacks (repo
// rule: pure functions, state passed/returned, no internal mutation).

import { useSyncExternalStore } from 'react';

export const TOKEN_KEY = 'oc_token';
export const BASE_KEY = 'oc_base';
/// Static fleet entry that routes to the server's own engine.
export const LOCAL_NODE = '__local__';
export const LOCAL_NODE_LABEL = '本机 (server 本机引擎)';

let state = {
  token: localStorage.getItem(TOKEN_KEY) || '',
  base: localStorage.getItem(BASE_KEY) ?? '',
  page: 'nodes', // 'nodes' | 'chat'
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

/// Persist + publish credentials. `base` is stored exactly as typed ('' =
/// same-origin); signing covers path+query only, so origin never enters the
/// canonical string.
export function setCredentials(token, base) {
  const cleanBase = String(base || '').trim().replace(/\/+$/, '');
  localStorage.setItem(TOKEN_KEY, token);
  localStorage.setItem(BASE_KEY, cleanBase);
  setState({ token, base: cleanBase, conn: 'init' });
}

export function clearCredentials() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(BASE_KEY);
  setState({ token: '', base: '', conn: 'init', nodes: [], preselectNode: null });
}

/// Origin-prefixing helper shared by api.js / time.js / sse.js.
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
