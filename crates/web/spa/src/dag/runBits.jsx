// runBits.jsx — small shared run display bits (status Tag, 执行节点 badge)
// used by BOTH runsTable.jsx and runDetail.jsx; kept in their own module so
// the table ↔ detail modules never import each other sideways.

import { Tag, Tooltip, Typography } from 'antd';
import { nodeBadgeText, runStatusLabel, runStatusTag } from '../dagProjection.js';
import { useStore } from '../store.js';

const { Text } = Typography;

/// Cancel is legal while the run can still transition (pending/running/
/// cancelling); terminal rows freeze (crates/dag transitions).
export const CANCELLABLE = ['pending', 'running', 'cancelling'];

export function RunStatusTag({ status }) {
  return <Tag color={runStatusTag(status)}>{runStatusLabel(status)}</Tag>;
}

/// 执行节点 badge: node name (fleet snapshot) with the raw id in a Tooltip;
/// unclaimed runs show the queue hint (pending) or — (post-hoc view).
export function NodeBadge({ nodeId, status }) {
  const { nodes } = useStore();
  if (!nodeId) {
    const unclaimed = status === 'pending' ? '任意节点排队中' : '—';
    return <Text type="secondary" style={{ fontSize: 12 }}>{nodeBadgeText(null, unclaimed)}</Text>;
  }
  const hit = (nodes || []).find((n) => n.id === nodeId);
  const label = hit && hit.name ? hit.name + ' · ' + nodeId.slice(0, 8) : nodeId;
  return (
    <Tooltip title={nodeId}>
      <Tag color="blue" style={{ marginInlineEnd: 0 }}>{label}</Tag>
    </Tooltip>
  );
}
