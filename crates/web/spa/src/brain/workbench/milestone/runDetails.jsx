import { Alert, Button, Collapse, Empty, Space, Table, Tag, Typography } from 'antd';
import { KIND_LABELS } from '../../../fleet/model.js';
import { LAYERED_STATUS } from '../layered/model.js';
import { ProblemView } from '../problem/view.jsx';
import { ProblemResults } from '../problem/results.jsx';

export function roundRows(history) {
  const rounds = new Map();
  for (const visit of history) {
    if (!rounds.has(visit.round)) rounds.set(visit.round, []);
    rounds.get(visit.round).push(visit);
  }
  return [...rounds].sort(([left], [right]) => right - left).map(([round, visits]) => ({
    round, visits,
    operations: visits.flatMap((visit) => visit.operations),
    layerCount: new Set(visits.map((visit) => visit.layer)).size,
  }));
}

export function RunDetails({ view, plan, history, onExecution }) {
  const nodes = new Map(plan.nodes.map((node) => [node.node_id, node]));
  const layers = plan.layers;
  const columns = [
    { title: '层级', key: 'layer', width: 170, render: (_, row) => `第 ${row.layer} 层 · ${layers[row.layer - 1]?.title || ''}` },
    { title: '执行节点', key: 'node', render: (_, row) => nodes.get(row.node_id)?.title || row.node_id },
    { title: '能力', dataIndex: 'execution_kind', key: 'kind', width: 110, render: (kind) => KIND_LABELS[kind] || kind || '—' },
    { title: '状态', dataIndex: 'status', key: 'status', width: 120, render: (status) => <Tag>{LAYERED_STATUS[status] || status}</Tag> },
    { title: '执行记录', key: 'execution', render: (_, row) => row.execution_id && row.status !== 'creating'
      ? <Button type="link" onClick={() => onExecution(row.execution_id)}>{row.execution_id}</Button>
      : <Typography.Text type="secondary">等待创建</Typography.Text> },
  ];
  return <div className="brain-run-details">
    <Typography.Title level={5}>{plan.title}</Typography.Title>
    {plan.objective && <Typography.Paragraph>{plan.objective}</Typography.Paragraph>}
    {view.run.summary && <Typography.Paragraph>交付摘要：{view.run.summary}</Typography.Paragraph>}
    {view.run.reflection && <Alert type="info" title="当前反思上下文" description={view.run.reflection} />}
    <ProblemView problem={view.problem} /><ProblemResults results={view.problem_results} />
    <Collapse items={roundRows(history).map(({ round, visits, operations, layerCount }) => ({
      key: String(round), label: `第 ${round} 轮 · ${layerCount} 层 · ${visits.reduce((count, visit) => count + visit.operations.length, 0)} 项执行`,
      children: <>
        {visits.filter((visit) => !visit.operations.length).map((visit) =>
          <Typography.Paragraph key={visit.activation} type="secondary">第 {visit.layer} 层 · 尚无执行记录</Typography.Paragraph>)}
        <Table size="small" rowKey="operation_id" pagination={false} scroll={{ x: 760 }} columns={columns}
          dataSource={operations}
          locale={{ emptyText: <Empty description="本轮尚无执行记录" /> }} />
        <Collapse ghost items={visits.map((visit) => ({ key: String(visit.activation),
          label: `第 ${visit.layer} 层 · 调度依据`, children: <>
            {visit.reason_summary && <Typography.Paragraph>{visit.reason_summary}</Typography.Paragraph>}
            {visit.reflection && <Alert type="info" title="本次整改上下文" description={visit.reflection} />}
            <Typography.Paragraph>依据：{visit.evidence_execution_ids?.join('、') || '初始计划'}</Typography.Paragraph>
            {visit.assignments?.length ? <Space orientation="vertical">{visit.assignments.map((assignment, index) => <details key={`${assignment.node_id}-${index}`}>
              <summary>{nodes.get(assignment.node_id)?.title || assignment.node_id} · 本次输入绑定</summary>
              <pre className="brain-json">{JSON.stringify(assignment.inputs || {}, null, 2)}</pre>
            </details>)}</Space> : null}
          </> }))} />
      </>,
    }))} />
  </div>;
}
