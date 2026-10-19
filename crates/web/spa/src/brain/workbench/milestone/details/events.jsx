import { Button, Empty, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { TimeText } from '../../../../ui/timeText.jsx';
import { DataValue, RawData, ReadableText } from '../../../../ui/runContent/value.jsx';
import { EVENT_LABELS } from './model.js';

function Event({ event, plan, onExecution }) {
  const [expanded, setExpanded] = useState(false);
  const layer = plan.layers[event.layer - 1];
  const task = plan.nodes.find((node) => node.node_id === event.node_id);
  return <article className="brain-event-card">
    <div className="brain-event-heading"><Space wrap><Typography.Text strong>{EVENT_LABELS[event.event_type] || event.event_type}</Typography.Text>
      {task && <Typography.Text>{task.title}</Typography.Text>}{event.round > 0 && <Tag>第 {event.round} 轮</Tag>}{layer && <Tag>{layer.title}</Tag>}</Space><TimeText ts={event.at_ms} /></div>
    {(event.user_input || event.reason_summary) && <ReadableText text={event.user_input || event.reason_summary} markdown />}
    {event.reflection && <section><Typography.Text strong>整改说明</Typography.Text><ReadableText text={event.reflection} markdown /></section>}
      {Object.entries(event.assessments || {}).map(([key, assessment]) => <div key={key}><Space wrap><Typography.Text strong>{plan.layers.find((item) => item.layer_id === key)?.title || key}</Typography.Text>
        <Tag color={assessment.met ? 'success' : 'error'}>{assessment.met ? '当时达标' : '当时未达标'}</Tag></Space><ReadableText text={assessment.reason} markdown /></div>)}
    {event.execution_id && <Button type="link" size="small" onClick={() => onExecution(event.execution_id)}>查看任务：{event.execution_id}</Button>}
    <Button type="link" size="small" onClick={() => setExpanded(!expanded)} aria-expanded={expanded}>{expanded ? '收起依据' : '查看依据'}</Button>
    {expanded && <div className="run-content-stack">
      {!!event.evidence_execution_ids?.length && <section><Typography.Text strong>依据执行</Typography.Text><Space wrap>{event.evidence_execution_ids.map((executionId) => <Button key={executionId} type="link" onClick={() => onExecution(executionId)}>{executionId}</Button>)}</Space></section>}
      {!!event.assignments?.length && <section><Typography.Text strong>本次任务输入绑定</Typography.Text><DataValue value={event.assignments} /></section>}
      {!!event.guidance?.length && <section><Typography.Text strong>下发引导</Typography.Text><DataValue value={event.guidance} /></section>}
      <RawData value={event} />
    </div>}
  </article>;
}

export function Events({ view, plan, onExecution }) {
  const [limit, setLimit] = useState(30);
  const rows = [...(view.events || [])].reverse();
  return <div className="run-content-stack">{!rows.length && <Empty description="暂无调度记录" />}
    {rows.slice(0, limit).map((event, index) => <Event key={event.seq ?? `${event.activation}:${event.event_type}:${index}`} event={event} plan={plan} onExecution={onExecution} />)}
    {rows.length > limit && <Button onClick={() => setLimit(limit + 30)}>加载更早记录（{limit} / {rows.length}）</Button>}
  </div>;
}
