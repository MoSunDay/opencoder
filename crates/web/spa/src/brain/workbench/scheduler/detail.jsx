import { Drawer, Select, Space, Typography } from 'antd';
import { ExecutionView } from '../../../fleet/detail.jsx';
import { KIND_LABELS } from '../../../fleet/model.js';
import { roundsOf, roundLabel } from '../v3Model.js';
import { executionCreated } from './rounds.jsx';

export function ExecutionDrawer({ view, operation, onSelect, onClose, onNotice }) {
  if (!operation) return null;
  const rounds = roundsOf(view); const operations = rounds.find((round) => round.round === operation.round)?.operations || [];
  const summary = { id: operation.execution_id, kind: operation.execution_kind, status: ['done', 'error', 'cancelled'].includes(operation.status) ? operation.status : 'running' };
  return <Drawer open destroyOnHidden title="能力执行明细" placement="right" size="82vw" onClose={onClose}>
    <Space wrap className="brain-execution-selectors">
      <Select aria-label="明细调度轮次" value={operation.round} options={rounds.map((round) => ({ value: round.round, label: roundLabel(round.round), disabled: !round.operations.some(executionCreated) }))} onChange={(number) => onSelect(rounds.find((round) => round.round === number).operations.find(executionCreated))} />
      <Select aria-label="选择执行" value={operation.execution_id} style={{ minWidth: 280, maxWidth: '100%' }} options={operations.map((op) => ({ value: op.execution_id, label: `${KIND_LABELS[op.execution_kind] || op.execution_kind} · ${op.execution_id}`, disabled: !executionCreated(op) }))} onChange={(id) => onSelect(operations.find((op) => op.execution_id === id))} />
      <Typography.Text code>{operation.execution_id}</Typography.Text>
    </Space>
    <ExecutionView key={operation.execution_id} executionRef={{ id: operation.execution_id, kind: operation.execution_kind }} summary={summary} onNotice={onNotice} managed />
  </Drawer>;
}
