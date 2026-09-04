// labels.jsx — one place for the project module's status → (color, 中文) maps
// and their Tag components, shared by goalsTab / milestonesTab / todosTab /
// todoDrawer so the wording can never drift between tabs. Status strings come
// from crates/store/src/project_types.rs (serde snake_case over the wire).

import { Tag } from 'antd';

export const GOAL_STATUS = {
  active: { color: 'green', label: '已进行' },
  archived: { color: 'default', label: '已归档' },
};

export const MILESTONE_STATUS = {
  planned: { color: 'default', label: '未开始' },
  in_progress: { color: 'blue', label: '进行中' },
  done: { color: 'green', label: '已完成' },
};

export const TODO_STATUS = {
  draft: { color: 'default', label: '草稿' },
  planned: { color: 'blue', label: '已规划' },
  running: { color: 'gold', label: '处理中' },
  done: { color: 'green', label: '完成' },
  failed: { color: 'red', label: '失败' },
};

export const RUN_KIND = {
  plan: 'Plan',
  execute: '执行',
};

export const RUN_STATUS = {
  running: { color: 'blue', label: '运行中' },
  done: { color: 'green', label: '完成' },
  failed: { color: 'red', label: '失败' },
  cancelled: { color: 'default', label: '已取消' },
};

function statusTag(map, status) {
  const meta = map[String(status || '')] || { color: 'default', label: String(status || '-') };
  return <Tag color={meta.color}>{meta.label}</Tag>;
}

export const GoalStatusTag = ({ status }) => statusTag(GOAL_STATUS, status);
export const MilestoneStatusTag = ({ status }) => statusTag(MILESTONE_STATUS, status);
export const TodoStatusTag = ({ status }) => statusTag(TODO_STATUS, status);
export const RunStatusTag = ({ status }) => statusTag(RUN_STATUS, status);

export const runKindLabel = (kind) => RUN_KIND[String(kind || '')] || String(kind || '-');
