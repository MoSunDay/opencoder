import { Alert, Button, Collapse, Drawer, InputNumber, Select, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../../api.js';
import { ExecutionView } from '../../../fleet/detail.jsx';
import { MilestoneCanvas } from './canvas.jsx';
import { visits } from './model.js';
import { convertPlan } from '../scheduler/model.js';
import { LAYERED_PHASES, LAYERED_STATUS } from '../layered/model.js';
export function MilestoneRunBody({ view, id, refresh, onNotice }) {
  const [executionId, setExecutionId] = useState(null); const [selectedVisit, setSelectedVisit] = useState(null);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const [budget, setBudget] = useState(null);
  const { run, plan } = view; const displayPlan = (plan.schema_version || view.schema_version) === 7 ? plan : convertPlan({ ...plan, schema_version: plan.schema_version || view.schema_version }); const history = visits(view); const visit = history.find((v) => v.activation === selectedVisit) || history[history.length - 1];
  const operations = visit?.operations || []; const allOps = view.operations || []; const execution = allOps.find((op) => op.execution_id === executionId);
  const current = allOps.filter((op) => op.activation === run.activation); const ended = current.filter((op) => ['done', 'error', 'cancelled'].includes(op.status)).length;
  const terminal = ['completed', 'failed', 'cancelled'].includes(run.phase);
  const command = async (action, input = {}) => {
    setBusy(true); setError('');
    try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action, input }); await refresh(); }
    catch (e) { setError(e.message); } finally { setBusy(false); }
  };
  const statuses = Object.fromEntries(plan.nodes.map((node) => {
    const ops = operations.filter((op) => op.node_id === node.node_id);
    const assessment = (view.events || []).find((e) => e.activation === visit?.activation && e.event_type === 'milestones_assessed')?.assessments?.[node.node_id];
    const label = assessment ? (assessment.met ? '已达标' : '需整改') : !ops.length ? '本次未派发' : ops.some((op) => !['done', 'error', 'cancelled'].includes(op.status)) ? '执行中' : ops.some((op) => op.status !== 'done') ? '执行有失败，等待判断' : '执行结束';
    return [node.node_id, label];
  }));
  const layerAssessment = (view.events || []).find((e) => e.activation === visit?.activation && e.event_type === 'milestones_assessed')?.assessments || {};
  const layerStatuses = Object.fromEntries(displayPlan.layers.map((layer) => [layer.layer_id, layerAssessment[layer.layer_id] ? (layerAssessment[layer.layer_id].met ? '已达标' : '需整改') : visit?.layer === displayPlan.layers.indexOf(layer) + 1 ? '当前执行' : '']));
  return <>
    <h2>{plan.title}</h2><p>{plan.objective}</p>
    {(error || run.error) && <Alert type="error" title={error || run.error} />}
    <Space wrap><Tag>第 {run.round} / {run.max_rounds} 轮</Tag><Tag>当前第 {run.layer || 1} 层</Tag><Tag>{LAYERED_PHASES[run.phase]}</Tag><Tag>层内执行已结束 {ended} / {current.length}</Tag><Tag>有效通过 {run.valid_layers} / {view.layers.length} 层</Tag>
      <Button disabled={busy || terminal} onClick={() => command(['paused', 'blocked'].includes(run.phase) ? 'resume' : 'pause')}>{['paused', 'blocked'].includes(run.phase) ? '继续调度' : '暂停调度'}</Button>
      <Button danger disabled={busy || terminal} onClick={() => command('cancel')}>取消运行</Button>
    </Space>
    {!view.problem && ['paused', 'blocked'].includes(run.phase) && <Space><InputNumber aria-label="新的轮次预算" min={run.round + 1} max={32} precision={0} value={budget} onChange={setBudget} /><Button disabled={busy || !budget} onClick={() => command('set_round_budget', { max_rounds: budget })}>调整预算</Button></Space>}
    {!!run.reflection && <Alert type="info" title="当前反思上下文" description={run.reflection} />}
    {!!run.summary && <Typography.Paragraph>交付摘要：{run.summary}</Typography.Paragraph>}
    <Select aria-label="选择历史层激活" style={{ minWidth: 360, margin: '16px 0' }} value={visit?.activation} onChange={setSelectedVisit}
      options={history.map((v) => ({ value: v.activation, label: `第 ${v.round} 轮 · 第 ${v.layer} 层 · ${v.decision_summary === 'reflect_and_return' ? '反思回退' : '正常推进'}` }))} />
    <div className="brain-milestone-preview"><MilestoneCanvas plan={displayPlan} capabilities={view.capabilities || []} statuses={statuses} layerStatuses={layerStatuses} onSelect={(selection) => { if (selection.type === 'node') { const op = operations.find((item) => item.node_id === selection.id); setExecutionId(op?.execution_id || null); } }} /></div>
    <h3>轮次、层级与能力执行</h3>
    <Collapse items={history.map((entry) => ({ key: entry.activation, label: `第 ${entry.round} 轮 · 第 ${entry.layer} 层 · ${entry.operations.length} 项执行`, children: <>
      <Typography.Paragraph>{entry.reason_summary}</Typography.Paragraph>
      {entry.reflection && <Alert type="info" title="本次整改上下文" description={entry.reflection} />}
      <Typography.Paragraph>依据：{entry.evidence_execution_ids.join('、') || '初始计划'}</Typography.Paragraph>
      {entry.operations.map((op) => <div className="brain-operation-card" key={op.operation_id}>
        <Space wrap><Typography.Text strong>{plan.nodes.find((n) => n.node_id === op.node_id)?.title}</Typography.Text><Tag>{op.execution_kind}</Tag><Tag>{LAYERED_STATUS[op.status]}</Tag></Space>
        <p>能力：{op.capability_id}</p><details><summary>本次输入绑定</summary><pre>{JSON.stringify(entry.assignments?.find((a) => a.node_id === op.node_id && a.capability_id === op.capability_id)?.inputs || {}, null, 2)}</pre></details><Typography.Text code copyable>{op.execution_id}</Typography.Text>
        <Button disabled={op.status === 'creating'} onClick={() => setExecutionId(op.execution_id)}>查看执行明细</Button>
      </div>)}
    </> }))} />
    <Drawer title="能力执行明细" open={!!execution} onClose={() => setExecutionId(null)} size="90vw" destroyOnHidden>
      {execution && <><Select aria-label="选择能力执行" value={executionId} onChange={setExecutionId} style={{ width: '100%', marginBottom: 12 }} options={allOps.map((op) => ({ value: op.execution_id, label: `第 ${op.round} 轮 · 第 ${op.layer} 层 · ${op.capability_id} · ${op.execution_id}` }))} />
        {execution.status === 'creating' ? <Alert type="info" title="执行尚未创建" /> : <ExecutionView executionRef={{ id: execution.execution_id, kind: execution.execution_kind }} managed onNotice={onNotice} />}</>}
    </Drawer>
  </>;
}
