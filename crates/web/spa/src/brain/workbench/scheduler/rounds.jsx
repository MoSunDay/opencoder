import { Button, Collapse, Empty, Space, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { capabilityFor, currentRound, roundLabel, roundsOf, V3_COLORS, V3_STATUS } from '../v3Model.js';
import { KIND_LABELS } from '../../../fleet/model.js';
export const executionCreated = (operation) => operation.execution_created ?? operation.status !== 'creating';

export function RoundsPanel({ view, onOpen }) {
  const rounds = roundsOf(view); const current = currentRound(view);
  const [expanded, setExpanded] = useState([String(current)]);
  useEffect(() => { setExpanded((old) => [...new Set([...old, String(current)])]); }, [current]);
  if (!rounds.length) return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未产生调度轮次" />;
  return <Collapse activeKey={expanded} onChange={setExpanded} items={rounds.map((round) => ({
    key: String(round.round),
    label: <Space><Typography.Text strong>{roundLabel(round.round)}</Typography.Text><Tag color={V3_COLORS[round.status]}>{{ completed: '本轮执行结束', running: '执行中', failed: '失败', cancelled: '已取消' }[round.status] || round.status}</Tag><Typography.Text type="secondary">{round.operations.length} 次能力调用</Typography.Text></Space>,
    children: <>
      {(round.decisions || []).map((decision) => <div className="brain-round-reason" key={decision.seq}>
        <Typography.Text strong>{decision.decision_summary === 'complete' ? '完成判断' : decision.decision_summary === 'dispatch' ? '调度理由' : '调度结果'}</Typography.Text>
        <Typography.Paragraph>{decision.reason_summary}</Typography.Paragraph>
        {!!decision.evidence_execution_ids?.length && <Space wrap><Typography.Text type="secondary">依据执行：</Typography.Text>{decision.evidence_execution_ids.map((id) => {
          const source = rounds.flatMap((r) => r.operations).find((operation) => operation.execution_id === id);
          return <Button key={id} size="small" type="link" disabled={!source || !executionCreated(source)} onClick={() => onOpen(source)}>{id}</Button>;
        })}</Space>}
      </div>)}
      <div className="brain-operation-list">{round.operations.map((operation) => {
        const capability = capabilityFor(view, operation); const created = executionCreated(operation);
        return <article className="brain-operation-card" key={operation.operation_id}>
          <Space wrap><Tag color={V3_COLORS[operation.status]}>{V3_STATUS[operation.status] || operation.status}</Tag><Tag>{KIND_LABELS[operation.execution_kind] || operation.execution_kind}</Tag><Typography.Text strong>{capability.target || capability.capability_id}</Typography.Text></Space>
          <Typography.Text type="secondary">能力 ID：{operation.capability_id}{capability.version ? ` · ${capability.version}` : ''}</Typography.Text>
          <Space wrap><Typography.Text type="secondary">{created ? '执行 ID' : '待创建 ID'}：</Typography.Text><Typography.Text code copyable>{operation.execution_id}</Typography.Text>
            <Button type="link" size="small" disabled={!created} onClick={() => onOpen(operation)}>查看执行 {operation.execution_id}</Button>
            {operation.cancel_requested && <Tag color="orange">已请求取消</Tag>}</Space>
        </article>;
      })}</div>
    </>,
  }))} />;
}
