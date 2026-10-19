import { Button, Empty, Space, Table, Tag, Typography } from 'antd';
import { useState } from 'react';
import { KIND_LABELS } from '../../../../fleet/model.js';
import { LAYERED_COLORS, LAYERED_STATUS } from '../../layered/model.js';
import { ReadableText } from '../../../../ui/runContent/value.jsx';
import { assessmentFor, roundRows } from './model.js';

export function Rounds({ view, plan, history, onExecution }) {
  const [chosen, setChosen] = useState(null);
  const rounds = roundRows(history);
  const round = chosen ?? view.run.round;
  const selected = rounds.find((row) => row.round === round);
  const columns = [
    { title: '任务', key: 'name', render: (_, row) => <div className="brain-task-identity"><Typography.Text strong>{plan.nodes.find((node) => node.node_id === row.node_id)?.title || row.node_id}</Typography.Text>
      {row.execution_id && <Typography.Text type="secondary" copyable>{row.execution_id}</Typography.Text>}</div> },
    { title: '能力类型', dataIndex: 'execution_kind', width: 110, render: (kind) => KIND_LABELS[kind] || kind },
    { title: '执行状态', dataIndex: 'status', width: 120, render: (status) => <Tag color={LAYERED_COLORS[status]}>{LAYERED_STATUS[status] || status}</Tag> },
    { title: '操作', key: 'action', width: 120, render: (_, row) => <Button type="link" disabled={!row.execution_id || row.status === 'creating'} onClick={() => onExecution(row.execution_id)}>查看执行详情</Button> },
  ];
  return <div className="run-content-stack">
    <div className="brain-round-selector" aria-label="选择执行轮次">{rounds.map((row) => <Button key={row.round} type={round === row.round ? 'primary' : 'default'} aria-pressed={round === row.round} onClick={() => setChosen(row.round)}>
      第 {row.round} 轮 · {row.layerCount} 层 · {row.operations.length} 项执行</Button>)}</div>
    {!selected && <Empty description="本轮尚未派发任务" />}
    {selected?.visits.map((visit) => {
      const layer = plan.layers[visit.layer - 1] || { layer_id: '', title: '' };
      const assessment = assessmentFor(view, layer, visit.activation);
      return <section className="brain-detail-section" key={visit.activation}>
        <Space wrap><Typography.Title level={5}>第 {visit.layer} 层 · {layer.title}</Typography.Title><Tag color={assessment.color}>{assessment.label}</Tag></Space>
        {assessment.reason && <ReadableText text={assessment.reason} markdown />}
        <Table size="small" rowKey="operation_id" pagination={false} scroll={{ x: 650 }} columns={columns} dataSource={visit.operations}
          locale={{ emptyText: '本层尚无执行记录' }} />
      </section>;
    })}
  </div>;
}
