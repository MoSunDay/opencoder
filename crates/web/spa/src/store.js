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
  page: 'nodes', // 'nodes' | 'chat' | 'dag' | 'team' | 'topics' | 'topic_detail'
  preselectNode: null, // node id the fleet tab asked chat to open
  nodes: [], // last fleet snapshot shared between tabs
  conn: 'init', // 'init' | 'ok' | 'fail'
  topicsTeamFilter: null, // team name the topics tab is filtered to (null = all)
  topicDetail: null, // {teamName, topicId} while page === 'topic_detail'
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
  setState({
    token: '', base: '', conn: 'init', nodes: [], preselectNode: null,
    topicsTeamFilter: null, topicDetail: null,
  });
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

/// 组队 tab "查看话题" → jump to the topics tab pre-filtered to that team
/// (same param-riding pattern as openChatForNode).
export function openTopicsForTeam(teamName) {
  setState({ page: 'topics', topicsTeamFilter: teamName || null, topicDetail: null });
}

/// Topics row "详情" → topic detail page; {teamName, topicId} ride the store.
export function openTopicDetail(teamName, topicId) {
  setState({ page: 'topic_detail', topicDetail: { teamName, topicId } });
}

/// Topic detail back button → topic list (keeps the team filter intact).
export function closeTopicDetail() {
  setState({ page: 'topics', topicDetail: null });
}

/// The topics tab's filter Select writes the same field
/// openTopicsForTeam arms, keeping one source of truth.
export function setTopicsTeamFilter(teamName) {
  setState({ topicsTeamFilter: teamName || null });
}
