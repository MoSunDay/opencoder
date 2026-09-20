// runMode.js — Agent 卡片 run_mode 字段的共享常量与文案（SPA 侧唯一事实源）。
//   operator（缺省）：会话跑在节点宿主机进程内（现状默认）；
//   agent：每轮会话在 runc 只读沙箱中运行（复用 DAG agent 步基建，加载该
//   Agent 的 skills/tools/提示词/CLI）。
// 线协议：run_mode 只随卡片下发/保存（POST /api/agents、PUT
// /api/agents/:name、GET /api/agents[/:name/meta]）；会话创建请求不带该
// 字段，worker 从目标 Agent 卡片读取。缺失一律收敛为 operator。
// 无 DOM、无 JSX —— 与 builtins.js 同层。

/// 新建/编辑表单与 Segmented 共用的选项（value + 展示文案）。
export const RUN_MODE_OPTIONS = [
  { value: 'operator', label: 'Operator · 宿主机' },
  { value: 'agent', label: 'Agent · runc 沙箱' },
];

/// 一行中文说明：表单 extra / tooltip 共用。
export const RUN_MODE_HINT = 'Operator：会话跑在节点宿主机进程内（默认）；Agent：每轮会话在 runc 只读沙箱中运行。';

/// 短徽标文案（chat 会话面板的小 Tag）。
export const RUN_MODE_BADGE = { operator: '宿主机', agent: '沙箱' };

/// 收敛任意（缺失/陌生/大小写不一致）值为 'operator' | 'agent'。
export function normalizeRunMode(value) {
  return value === 'agent' ? 'agent' : 'operator';
}

/// 徽标文案：normalize 后查表，缺失兜底 operator。
export function runModeBadge(value) {
  return RUN_MODE_BADGE[normalizeRunMode(value)];
}

/// Meta tab 的完整 Tag 文案。
export function runModeTagText(value) {
  return normalizeRunMode(value) === 'agent' ? 'agent · 沙箱' : 'operator · 宿主机';
}
