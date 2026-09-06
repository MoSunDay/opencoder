// statusTag.jsx — THE single status → visual mapping for the whole console.
// One flat table STATUS_META absorbs every status vocabulary that used to be
// hand-written per panel (fleet/model.js 执行状态, todoRunsPanel 工作流/TODO 项,
// project/labels.jsx 项目目标/里程碑/TODO/运行, nodes 在线态), so color + 中文
// wording can never drift between panels again. Function components only.
//
// Compatibility seam: fleet/model.js re-exports STATUS_COLORS / STATUS_LABELS
// from here, so every legacy import path keeps the exact same colors/labels
// (DOM tests guard that copy) while the table itself lives in one place.

import { Tag } from 'antd';

/// status → { color, label }. Sources, absorbed verbatim where they existed:
/// - 执行 (was fleet/model.js): pending/running/idle/cancelling/interrupted/
///   cancelled/done/error — colors + 文案 unchanged.
/// - TODO 工作流/TODO 项 (was todoRunsPanel + crates/todos/src/types.rs):
///   suspended/completed/failed + the item lifecycle states.
/// - 项目 (was project/labels.jsx): draft/planned/in_progress/active/archived;
///   running/done/failed fold into the shared rows above (planned → cyan,
///   draft → default per the unified palette).
/// - 节点在线态 (was nodes.jsx/executions.jsx): online/offline.
export const STATUS_META = {
  // 执行状态（fleet/model.js 原表，逐字吸收）
  pending: { color: 'default', label: '等待节点确认' },
  running: { color: 'processing', label: '运行中' },
  idle: { color: 'blue', label: '等待继续' },
  cancelling: { color: 'warning', label: '取消中' },
  interrupted: { color: 'orange', label: '已中断' },
  cancelled: { color: 'default', label: '已取消' },
  done: { color: 'success', label: '已完成' },
  error: { color: 'error', label: '失败' },
  // TODO 工作流状态（crates/todos WorkflowStatus）
  suspended: { color: 'warning', label: '已挂起' },
  completed: { color: 'success', label: '已完成' },
  failed: { color: 'error', label: '失败' },
  // TODO 项状态（crates/todos TodoStatus 的生命周期扩展态）
  candidate_ready: { color: 'gold', label: '候选就绪' },
  accepting: { color: 'cyan', label: '验收中' },
  needs_revision: { color: 'orange', label: '待修改' },
  passed: { color: 'success', label: '已通过' },
  invalidated: { color: 'default', label: '已失效' },
  recovering: { color: 'blue', label: '恢复中' },
  // 项目模块（crates/store project_types.rs）
  draft: { color: 'default', label: '草稿' },
  planned: { color: 'cyan', label: '已规划' },
  in_progress: { color: 'blue', label: '进行中' },
  active: { color: 'green', label: '已进行' },
  archived: { color: 'default', label: '已归档' },
  // 节点在线态
  online: { color: 'success', label: '在线' },
  offline: { color: 'error', label: '离线' },
};

/// Legacy shape for fleet/model.js importers: status → antd Tag color token.
export const STATUS_COLORS = Object.fromEntries(
  Object.entries(STATUS_META).map(([status, meta]) => [status, meta.color]),
);

/// Legacy shape for fleet/model.js importers: status → 中文文案.
export const STATUS_LABELS = Object.fromEntries(
  Object.entries(STATUS_META).map(([status, meta]) => [status, meta.label]),
);

/// statusColor(status) → antd Tag color token; unknown statuses fall back to
/// the neutral default token.
export function statusColor(status) {
  const meta = STATUS_META[String(status || '')];
  return meta ? meta.color : 'default';
}

/// statusLabel(status) → 中文文案; unknown statuses render the raw string,
/// missing statuses render '-'.
export function statusLabel(status) {
  const key = String(status || '');
  const meta = STATUS_META[key];
  return meta ? meta.label : key || '-';
}

/// StatusTag — the one status Tag of the console. `label` / `color` override
/// the table lookup (e.g. nodes render a dynamic resource_error label);
/// unknown statuses fall back to default + the raw status string.
export function StatusTag({ status, label, color }) {
  return <Tag color={color || statusColor(status)}>{label || statusLabel(status)}</Tag>;
}
