import { MilestoneRunBody } from '../milestone/run.jsx';
// run.jsx — the v4 workbench body: layered canvas, layer barrier progress,
// layer decisions and the v4 event journal. v3 runs never reach this module.
import { Alert, Button, Collapse, Drawer, Select, Progress, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../../api.js';
import { KIND_LABELS } from '../../../fleet/model.js';
import { ExecutionView } from '../../../fleet/detail.jsx';
import { TimeText } from '../../../ui/timeText.jsx';
import { LayerCanvas } from './canvas.jsx';
import { LayeredEvents } from './events.jsx';
import { LayerRounds } from './rounds.jsx';
import { LAYERED_COLORS, LAYERED_PHASES, barrier, layeredPhase, planOf, terminalPhase } from './model.js';
import './style.css';
import { ProblemView } from '../problem/view.jsx';
import { ProblemResults } from '../problem/results.jsx';

// Execution events refresh the view; periodic reads also report connection failures.
const CONNECTION_TEXT = {
  live: '正在实时同步', open: '正在实时同步', connecting: '正在同步',
  reconnecting: '正在重连', closed: '运行已结束', polling: '定时同步', failed: '推送不可用，定时同步',
};

function CapabilityList({ capabilities = [] }) {
  if (!capabilities.length) return <Typography.Text type="secondary">本次运行没有记录关联能力</Typography.Text>;
  return <div className="brain-layer-capabilities">{capabilities.map((capability) => <section className="brain-operation-card" key={capability.capability_id}>
    <Space wrap><Tag>{KIND_LABELS[capability.kind] || capability.kind}</Tag><Typography.Text strong>{capability.target || capability.capability_id}</Typography.Text>{capability.version ? <Tag>v{capability.version}</Tag> : null}</Space>
    <Typography.Text code>{capability.capability_id}</Typography.Text>
  </section>)}</div>;
}

function LegacyRunBody({ view, id, connection, refresh, onNotice }) {
  const [busy, setBusy] = useState(false); const [commandError, setCommandError] = useState(''); const [selected, setSelected] = useState(null);
  const [executionId, setExecutionId] = useState(null);
  const operations = view.operations || [];
  const execution = operations.find((op) => op.execution_id === executionId);
  const selectNode = (nodeId) => { setSelected(nodeId); const op = operations.filter((op) => op.node_id === nodeId).sort((a, b) => b.attempt - a.attempt)[0]; setExecutionId(op?.execution_id || null); };
  const run = view.run || {}; const plan = planOf(view); const phase = layeredPhase(view); const progress = barrier(view);
  const command = async (action) => {
    setBusy(true); setCommandError('');
    try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action }); await refresh(); }
    catch (error) { setCommandError(error.message); }
    finally { setBusy(false); }
  };
  return <>
    {(commandError || view.error) && <Alert type="error" showIcon title={commandError || view.error} />}
    {run.error && ['blocked', 'failed'].includes(phase) && <Alert type="error" showIcon title="运行阻塞或失败" description={run.error} />}
    <div className="brain-run-header">
      <div>
        <Typography.Title level={4} ellipsis={{ rows: 2, expandable: 'collapsible', symbol: (expanded) => (expanded ? '收起目标' : '展开目标') }}>{plan.title || run.run_id}</Typography.Title>
        {!!plan.objective && <Typography.Paragraph className="brain-layer-objective">{plan.objective}</Typography.Paragraph>}
        <Space wrap>
          <Tag color="purple">分层能力计划</Tag>
          <Tag color={LAYERED_COLORS[phase]}>{LAYERED_PHASES[phase] || phase}</Tag>
          <Tag>层屏障 {progress.label}</Tag>
          {run.parent && <Tag color="geekblue">子运行 · 父层 {run.parent.layer}</Tag>}
          {plan.todoId && <Tag color="cyan">TODO {plan.todoId}</Tag>}
          <Typography.Text type="secondary">{CONNECTION_TEXT[connection] || connection}</Typography.Text>
          <TimeText ts={run.updated_at} />
        </Space>
      </div>
      <Space>
        <Button disabled={busy || terminalPhase(phase)} onClick={() => command(phase === 'paused' ? 'resume' : 'pause')}>{phase === 'paused' ? '继续调度' : '暂停调度'}</Button>
        <Button danger disabled={busy || terminalPhase(phase)} onClick={() => command('cancel')}>取消调度</Button>
      </Space>
    </div>
    <section className="brain-layer-barrier" aria-label="层屏障进度">
      <Progress percent={progress.percent} size="small" format={() => progress.label} />
      <Space wrap>
        <Typography.Text type="secondary">已完成 {progress.completed} / 共 {progress.total} 层{progress.dispatched ? ` · 已派发至第 ${progress.dispatched} 层` : ''}</Typography.Text>
        {!!run.summary && <Typography.Text type="secondary">交付摘要：{run.summary}</Typography.Text>}
      </Space>
    </section>
    <LayerCanvas view={view} selected={selected} onSelect={selectNode} />
    <section className="brain-rounds">
      <Typography.Title level={5}>分层决策与执行</Typography.Title>
      <LayerRounds id={id} view={view} onExecution={setExecutionId} />
    </section>
    <Collapse items={[
      { key: 'events', label: `分层事件（${(view.events || []).length}）`, children: <LayeredEvents view={view} /> },
      { key: 'capabilities', label: `本次运行关联能力（${(view.capabilities || []).length}）`, children: <CapabilityList capabilities={view.capabilities} /> },
    ]} />
    <Drawer open={!!execution} onClose={() => setExecutionId(null)} title="能力运行明细" size="90vw" destroyOnHidden>
      {execution && <><Select aria-label="选择执行尝试" style={{ width: '100%', marginBottom: 16 }} value={executionId} onChange={setExecutionId} options={operations.map((op) => ({ value: op.execution_id, label: `第 ${op.layer} 层 · ${plan.nodes.find((n) => n.node_id === op.node_id)?.title || op.node_id} · 第 ${op.attempt} 次尝试` }))} />
        {execution.status === 'creating' ? <Alert type="info" title="等待派发，执行尚未创建" /> : <ExecutionView key={executionId} executionRef={{ id: executionId, kind: execution.execution_kind }} managed onNotice={onNotice} />}</>}
    </Drawer>
  </>;
}

export function LayeredRunBody(props) {
  return <><ProblemView problem={props.view.problem} /><ProblemResults results={props.view.problem_results} />
    {props.view.schema_version >= 5 ? <MilestoneRunBody {...props} /> : <LegacyRunBody {...props} />}</>;
}
