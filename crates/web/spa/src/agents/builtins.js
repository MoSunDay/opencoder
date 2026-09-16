// builtins.js — 内置 subagent 调度角色常量（SPA 侧唯一事实源镜像）。
//
// builtin 角色定义在 crates/core/src/agent/mod.rs::builtin_agents()，其中
// AgentMode::Primary 的只有 act/plan/command（workflow 虽为 Primary 但属
// TODO 内部调度器，被各消费方显式排除；explore/build/sidecar 是 subagent）。
// GET /api/agents 只返回注册卡，需要「可选 Primary Agent / 调度目标」语义的
// 消费方（todoEditor 的 allowed、brain 能力编辑器的 agent 目标）在本层把
// 这三个内置角色并入注册卡列表。
export const BUILTIN_PRIMARY_AGENTS = ['act', 'plan', 'command'];

/// 内置 Primary Agent 的菜单卡片（name + 一行描述），描述镜像
/// crates/core/src/agent/mod.rs::builtin_agents()，供 `@`/`/agent` 菜单
/// 展示（注册卡的 description 由 GET /api/agents 下发）。
export const BUILTIN_PRIMARY_AGENT_CARDS = [
  {
    name: 'act',
    description: 'Default execution agent. Orchestrates work via bash and subagents.',
  },
  {
    name: 'plan',
    description: 'Read-only plan agent. Explores and answers questions; mutating operations are intercepted.',
  },
  {
    name: 'command',
    description: 'One-shot single-turn agent. Runs a single prompt to completion without interactive follow-up.',
  },
];

/// 内置角色在前、注册卡在后，去掉与内置重名的项（builtin 名字天然不可被
/// file 卡遮蔽，重名只可能来自脏数据）。
export function mergeBuiltinPrimaryAgents(registered) {
  const names = (Array.isArray(registered) ? registered : []).filter(
    (name) => typeof name === 'string' && name && !BUILTIN_PRIMARY_AGENTS.includes(name),
  );
  return [...BUILTIN_PRIMARY_AGENTS, ...names];
}

/// 卡片版合并（chat.jsx 的 `@` 菜单目录）：内置卡片在前、注册卡在后，
/// 内置重名项丢弃。`registered` 是 GET /api/agents 里已按 `primary` 过滤
/// 的卡片（name + description）。
export function mergeBuiltinPrimaryAgentCards(registered) {
  const cards = (Array.isArray(registered) ? registered : []).filter(
    (a) => a && typeof a.name === 'string' && a.name && !BUILTIN_PRIMARY_AGENTS.includes(a.name),
  );
  return [...BUILTIN_PRIMARY_AGENT_CARDS.map((c) => ({ ...c })), ...cards];
}
