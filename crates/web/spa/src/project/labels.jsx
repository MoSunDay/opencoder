// labels.jsx — project-module status display. The status → (color, 中文)
// mapping itself now lives in ONE console-wide table (src/ui/statusTag.jsx,
// absorbed in iteration 3); these named Tag components stay as the module's
// stable seam so goalsTab / milestonesTab / todosTab / todoDrawer keep their
// imports, and non-status labels (run kind) stay local. Status strings come
// from crates/store/src/project_types.rs (serde snake_case over the wire).

import { Tag } from 'antd';
import { StatusTag } from '../ui/statusTag.jsx';

export const GoalStatusTag = ({ status }) => <StatusTag status={status} />;
export const MilestoneStatusTag = ({ status }) => <StatusTag status={status} />;
export const TodoStatusTag = ({ status }) => <StatusTag status={status} />;
export const RunStatusTag = ({ status }) => <StatusTag status={status} />;

/// executor_kind → { color, label } (wire strings from
/// crates/store/src/project_types.rs serde snake_case). Same named-Tag seam
/// as the status tags above: todosTab / todoDrawer import the component, the
/// table stays the single mapping.
export const EXECUTOR_META = {
  agent: { color: 'blue', label: '单Agent' },
  team: { color: 'purple', label: '团队' },
  dag: { color: 'cyan', label: 'DAG' },
  brain: { color: 'gold', label: '大脑' },
};

export const ExecutorTag = ({ kind }) => {
  const meta = EXECUTOR_META[String(kind || '')] || { color: 'default', label: String(kind || '-') };
  return <Tag color={meta.color}>{meta.label}</Tag>;
};

export const RUN_KIND = {
  plan: 'Plan',
  execute: '执行',
};

export const runKindLabel = (kind) => RUN_KIND[String(kind || '')] || String(kind || '-');
