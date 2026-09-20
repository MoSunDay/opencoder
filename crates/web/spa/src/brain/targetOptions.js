// targetOptions.js — TYPE → NAME cascade for the capability editor (pure
// functions, no React). Each 执行类型 kind maps to one listing endpoint; the
// responses differ in shape, so targetOptionsFrom normalizes them all into
// [{value,label}] with defensive filtering (non-arrays / nameless entries
// are dropped instead of crashing the editor).

import { mergeBuiltinPrimaryAgents } from '../agents/builtins.js';

export const TARGET_ENDPOINTS = {
  agent: '/api/agents',
  operator: '/api/agents',
  team: '/api/teams',
  dag: '/api/dag/defs',
  todos: '/api/todo/templates',
};

const trimmed = (value) => (value == null ? '' : String(value).trim());
const list = (value) => (Array.isArray(value) ? value : []);
const toOptions = (names) => names.map((name) => ({ value: name, label: name }));
const pluck = (items, field) => list(items)
  .filter((item) => item && typeof item === 'object')
  .map((item) => trimmed(item[field]))
  .filter(Boolean);

function todosOptions(payload) {
  return list(payload?.templates).flatMap((template) => {
    if (!template || typeof template !== 'object' || !trimmed(template.name)) return [];
    const name = trimmed(template.name);
    return list(template.versions)
      .filter((version) => version && typeof version === 'object')
      .map((version) => trimmed(version.version))
      .filter(Boolean)
      .map((version) => `${name}/${version}`);
  });
}

export function targetOptionsFrom(kind, payload) {
  // /api/agents 只返回注册卡；brain 步骤可合法调度内置 primary 角色
  // （playbook 规范示例即 target 'act'），与 todoEditor 的 allowed 同语义
  // ——这里选的是调度执行器目标，不是 Agent 对话模式的能力选择面（后者
  // 只列注册卡，分界见 agents/builtins.js 头注释）。
  if (kind === 'agent' || kind === 'operator') return toOptions(mergeBuiltinPrimaryAgents(pluck(payload?.agents, 'name')));
  if (kind === 'team') return toOptions(pluck(payload?.teams, 'name'));
  if (kind === 'dag') {
    const defs = Array.isArray(payload) ? payload : payload?.defs;
    return toOptions(pluck(defs, 'id'));
  }
  if (kind === 'todos') return toOptions(todosOptions(payload));
  return [];
}

export async function fetchTargetOptions(apiGet, kind) {
  const endpoint = TARGET_ENDPOINTS[kind];
  if (!endpoint) return [];
  return targetOptionsFrom(kind, await apiGet(endpoint));
}
