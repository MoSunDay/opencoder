// builtins.js — 内置 runtime 执行角色常量（core 唯一事实源的 SPA 镜像）。
//
// 两类分界：builtin（act/plan/command 等）是 agent loop 的 runtime 执行
// 角色，定义在 crates/core/src/agent/mod.rs::builtin_agents()，与注册
// Agent 卡（opencoder-agent 节点注册、NFS 分发、GET /api/agents 下发）是
// 完全不同的两类物。builtin 中 AgentMode::Primary 的只有 act/plan/command
// （workflow 虽为 Primary 但属 TODO 内部调度器，被各消费方显式排除；
// explore/build/sidecar 是 subagent）。
//
// 本层 merge（内置角色并入注册卡列表）的合法用途仅限：调度执行器目标
// （todoEditor 的 allowed、brain 能力编辑器的 targetOptions）与 Operator
// 会话切换面（chat.jsx Operator 模式的 `@` 菜单与选中兜底）；Agent 对话
// 模式的「执行 Agent」下拉禁用本层合并——只列 GET /api/agents 的注册卡。
export const BUILTIN_PRIMARY_AGENTS = ['act', 'plan', 'command'];

/// 拥有独立控制头的内置角色：`/act` 与 `/plan`（commandMenu 的
/// COMMAND_CATALOG 同款）。其余切换目标（command 与全部自定义注册卡）走
/// 通用 `/agent <name>` 头 —— 与 crates/session/src/control_cmd.rs 的
/// 解析面保持一致。
export const BUILTIN_AGENT_HEADS = ['act', 'plan'];

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

/// 卡片版合并（chat.jsx Operator 模式的 `@` 菜单目录与选中兜底）：内置
/// 卡片在前、注册卡在后，内置重名项丢弃。Agent 模式的「执行 Agent」下拉
/// 不走本合并——只列 GET /api/agents 注册卡。`registered` 是 GET /api/agents
/// 里已按 `primary` 过滤的卡片（name + description）。
export function mergeBuiltinPrimaryAgentCards(registered) {
  const cards = (Array.isArray(registered) ? registered : []).filter(
    (a) => a && typeof a.name === 'string' && a.name && !BUILTIN_PRIMARY_AGENTS.includes(a.name),
  );
  return [...BUILTIN_PRIMARY_AGENT_CARDS.map((c) => ({ ...c })), ...cards];
}
