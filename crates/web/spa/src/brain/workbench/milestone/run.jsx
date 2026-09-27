import { Alert, Button, Drawer, Input, InputNumber, Space, Tag } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../../api.js';
import { ExecutionView } from '../../../fleet/detail.jsx';
import { MilestoneCanvas } from './canvas.jsx';
import { visits } from './model.js';
import { RunDetails } from './runDetails.jsx';
import { convertPlan } from '../scheduler/model.js';
import { LAYERED_PHASES } from '../layered/model.js';

export function MilestoneRunBody({ view, id, refresh, onNotice }) {
  const [executionId, setExecutionId] = useState(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const [budget, setBudget] = useState(null);
  const [humanInput, setHumanInput] = useState('');
  const inputBytes = new TextEncoder().encode(humanInput.trim()).length;
  const { run, plan } = view;
  const displayPlan = (plan.schema_version || view.schema_version) === 7 ? plan : convertPlan({ ...plan, schema_version: plan.schema_version || view.schema_version });
  const history = visits(view);
  const visit = history.find((entry) => entry.activation === run.activation) || history[history.length - 1];
  const operations = visit?.operations || [];
  const execution = (view.operations || []).find((op) => op.execution_id === executionId);
  const terminal = ['completed', 'failed', 'cancelled'].includes(run.phase);
  const command = async (action, input = {}) => {
    setBusy(true); setError('');
    try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action, input }); await refresh(); }
    catch (e) { setError(e.message); } finally { setBusy(false); }
  };
  const assessments = (view.events || []).find((event) => event.activation === visit?.activation && event.event_type === 'milestones_assessed')?.assessments || {};
  const statuses = Object.fromEntries(plan.nodes.map((node) => {
    const attempts = operations.filter((op) => op.node_id === node.node_id);
    const assessment = assessments[node.node_id];
    const label = assessment ? (assessment.met ? '已达标' : '需整改') : !attempts.length ? '本次未派发' : attempts.some((op) => !['done', 'error', 'cancelled'].includes(op.status)) ? '执行中' : attempts.some((op) => op.status !== 'done') ? '执行有失败，等待判断' : '执行结束';
    return [node.node_id, label];
  }));
  const layerStatuses = Object.fromEntries(displayPlan.layers.map((layer, index) => [layer.layer_id,
    assessments[layer.layer_id] ? (assessments[layer.layer_id].met ? '已达标' : '需整改') : visit?.layer === index + 1 ? '当前执行' : '']));
  const openExecution = (executionIdToOpen) => { if (executionIdToOpen) { setExecutionId(executionIdToOpen); setDetailsOpen(true); } };
  const sendInput = async (message) => {
    if (!message.trim()) return false;
    if (new TextEncoder().encode(message.trim()).length > 4096) { setError('人工输入不能超过 4096 字节'); return false; }
    setBusy(true); setError('');
    try {
      await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/inputs`, { text: message.trim() });
      try { await refresh(); } catch (e) { setError(`输入已记录，刷新失败：${e.message}`); }
      return true;
    } catch (e) { setError(e.message); return false; } finally { setBusy(false); }
  };
  const submitInput = async () => { if (await sendInput(humanInput)) setHumanInput(''); };
  return <>
    <div className="brain-run-status">
      <Space wrap><Tag>第 {run.round} / {run.max_rounds} 轮</Tag><Tag>当前第 {run.layer || 1} 层</Tag><Tag>{LAYERED_PHASES[run.phase] || run.phase}</Tag></Space>
      <Space wrap>
        <Button disabled={busy || terminal} onClick={() => command(['paused', 'blocked'].includes(run.phase) ? 'resume' : 'pause')}>{['paused', 'blocked'].includes(run.phase) ? '继续调度' : '暂停调度'}</Button>
        <Button danger disabled={busy || terminal} onClick={() => command('cancel')}>取消运行</Button>
        <Button aria-label="打开大脑对话" onClick={() => { setExecutionId(null); setDetailsOpen(true); }}>与大脑对话</Button>
        <Button onClick={() => { setExecutionId(null); setDetailsOpen(true); }}>查看详情</Button>
      </Space>
    </div>
    <div className="brain-milestone-preview"><MilestoneCanvas plan={displayPlan} capabilities={view.capabilities || []} statuses={statuses} layerStatuses={layerStatuses}
      onSelect={(selection) => { if (selection.type === 'node') openExecution(operations.find((op) => op.node_id === selection.id)?.execution_id); }} /></div>
    <Drawer placement="right" title={execution ? '能力执行明细' : '计划运行详情'} open={detailsOpen} onClose={() => { setDetailsOpen(false); setExecutionId(null); }}
      size="min(900px, 100vw)" destroyOnHidden>
      {(error || run.error) && <Alert type="error" title={error || run.error} />}
      {execution ? <>
        <Button onClick={() => setExecutionId(null)} style={{ marginBottom: 16 }}>返回轮次列表</Button>
        <ExecutionView key={execution.execution_id} executionRef={{ id: execution.execution_id, kind: execution.execution_kind }}
          managed allowGuidance={!terminal} onGuidance={sendInput} onNotice={onNotice} />
      </> : <>
        <div className="brain-human-input"><Space orientation="vertical" style={{ width: '100%' }}>
          <Input.TextArea aria-label="大脑人工输入" disabled={terminal} value={humanInput} onChange={(e) => setHumanInput(e.target.value)} rows={3}
            placeholder="补充信息或调整调度要求；信息将作为最新事件交给大脑" />
          <span className={inputBytes > 4096 ? 'brain-input-count over-limit' : 'brain-input-count'}>{inputBytes} / 4096 字节</span>
          <Button aria-label="发送大脑输入" type="primary" loading={busy} disabled={terminal || !humanInput.trim() || inputBytes > 4096} onClick={submitInput}>发送给大脑</Button>
        </Space></div>
        {(view.events || []).filter((event) => ['human_input', 'guidance_processed'].includes(event.event_type)).slice(-20).map((event) =>
          <div key={event.seq} className="brain-human-message"><strong>{event.event_type === 'human_input' ? '你' : '大脑'}：</strong>{event.user_input || event.reason_summary}</div>)}
        {!view.problem && ['paused', 'blocked'].includes(run.phase) && <Space style={{ marginBottom: 16 }}>
          <InputNumber aria-label="新的轮次预算" min={run.round + 1} max={32} precision={0} value={budget} onChange={setBudget} />
          <Button disabled={busy || !budget} onClick={() => command('set_round_budget', { max_rounds: budget })}>调整预算</Button>
        </Space>}
        <RunDetails view={view} plan={displayPlan} history={history} onExecution={openExecution} />
      </>}
    </Drawer>
  </>;
}
