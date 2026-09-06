// labels.jsx — project-module status display. The status → (color, 中文)
// mapping itself now lives in ONE console-wide table (src/ui/statusTag.jsx,
// absorbed in iteration 3); these named Tag components stay as the module's
// stable seam so goalsTab / milestonesTab / todosTab / todoDrawer keep their
// imports, and non-status labels (run kind) stay local. Status strings come
// from crates/store/src/project_types.rs (serde snake_case over the wire).

import { StatusTag } from '../ui/statusTag.jsx';

export const GoalStatusTag = ({ status }) => <StatusTag status={status} />;
export const MilestoneStatusTag = ({ status }) => <StatusTag status={status} />;
export const TodoStatusTag = ({ status }) => <StatusTag status={status} />;
export const RunStatusTag = ({ status }) => <StatusTag status={status} />;

export const RUN_KIND = {
  plan: 'Plan',
  execute: '执行',
};

export const runKindLabel = (kind) => RUN_KIND[String(kind || '')] || String(kind || '-');
