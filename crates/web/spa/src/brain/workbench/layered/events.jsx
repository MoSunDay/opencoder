// events.jsx — the v4 run journal carried inside the locked layered view:
// layer, node, attempt, event type and the decision / reason summary.
import { Empty, Space, Tag, Typography } from 'antd';
import { TimeText } from '../../../ui/timeText.jsx';
import { decisionLabel, eventLabel, eventRows, eventSummary, layerLabel } from './model.js';

const VISIBLE = 200;

function eventDetail(row) {
  const detail = {};
  if (row.decisionSummary) detail.decision = decisionLabel(row.decisionSummary);
  if (row.reasonSummary) detail.reason = row.reasonSummary;
  if (row.executionId) detail.execution_id = row.executionId;
  if (row.evidence.length) detail.evidence_execution_ids = row.evidence;
  if (row.atMs) detail.at_ms = row.atMs;
  return detail;
}

export function LayeredEvents({ view }) {
  const rows = eventRows(view);
  if (!rows.length) return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="该 v4 运行还没有事件" />;
  const shown = rows.slice(-VISIBLE);
  return <div className="brain-events brain-layer-events" aria-label="分层运行事件">
    {rows.length > shown.length && <Typography.Text type="secondary">仅显示最近 {shown.length} / {rows.length} 条事件</Typography.Text>}
    {shown.map((row) => <details key={row.seq}>
      <summary>
        <span>#{row.seq}</span>
        <Tag>{layerLabel(row.layer)}</Tag>
        <Tag color="geekblue">{eventLabel(row.eventType)}</Tag>
        {row.nodeId ? <Typography.Text code>{row.nodeId}</Typography.Text> : null}
        {row.attempt !== null ? <span>尝试 {row.attempt}</span> : null}
        <span className="brain-layer-event-summary">{eventSummary(row)}</span>
        {row.atMs ? <TimeText ts={row.atMs} /> : null}
        {!!row.evidence.length && <Space size={4} wrap><Typography.Text type="secondary">依据 {row.evidence.length} 项</Typography.Text></Space>}
      </summary>
      <pre className="brain-json">{JSON.stringify(eventDetail(row), null, 2)}</pre>
    </details>)}
  </div>;
}
