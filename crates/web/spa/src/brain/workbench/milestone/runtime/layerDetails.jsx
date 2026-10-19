import { Button, Empty, Select, Space, Tag, Typography } from 'antd';
import { ExecutionView } from '../../../../fleet/detail.jsx';
import { LAYERED_STATUS } from '../../layered/model.js';
import { canOpenExecution, latestExecutions, layerExecutions } from './model.js';

export function LayerDetails({ layerId, view, plan, executionId, onExecution, terminal, onGuidance, onNotice }) {
  const layer = plan.layers.find((item) => item.layer_id === layerId);
  const rows = layerExecutions(plan, view.operations || [], layerId);
  const execution = rows.find((row) => row.execution_id === executionId);
  const nodeTitle = (row) => plan.nodes.find((node) => node.node_id === row.node_id)?.title || row.node_id;
  return <div className="brain-layer-details">
    {layer?.task && <Typography.Paragraph type="secondary">{layer.task}</Typography.Paragraph>}
    {!!rows.length && <Select aria-label="选择层执行记录" className="brain-layer-execution-select" value={executionId || undefined}
      placeholder="选择任务或历史轮次" onChange={onExecution}
      options={rows.map((row) => ({ value: row.execution_id || row.operation_id, disabled: !canOpenExecution(row),
        label: `${nodeTitle(row)} · 第 ${row.round || 1} 轮 · ${LAYERED_STATUS[row.status] || row.status}` }))} />}
    {execution ? <>
      <Typography.Title level={5}>{nodeTitle(execution)}</Typography.Title>
      <Space wrap className="brain-layer-execution-heading"><Tag>第 {execution.round || 1} 轮</Tag>
        <Typography.Text copyable code>{execution.execution_id}</Typography.Text>
        {latestExecutions(rows).length > 1 && <Button onClick={() => onExecution(null)}>本层全部任务</Button>}
      </Space>
      <ExecutionView key={execution.execution_id} executionRef={{ id: execution.execution_id, kind: execution.execution_kind }}
        presentation="brain" managed allowGuidance={!terminal} onGuidance={onGuidance} onNotice={onNotice} />
    </> : rows.length ? <div className="brain-layer-execution-list">{latestExecutions(rows).map((row) =>
      <Button key={row.operation_id} disabled={!canOpenExecution(row)} onClick={() => onExecution(row.execution_id)}>
        <span>{nodeTitle(row)}</span><Tag>{LAYERED_STATUS[row.status] || row.status}</Tag><span>{canOpenExecution(row) ? '查看执行详情 →' : '等待创建'}</span>
      </Button>)}</div> : <Empty description="本层尚未派发，执行开始后可在这里查看详情" />}
  </div>;
}
