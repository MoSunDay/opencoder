import { Alert, Breadcrumb, Button, Empty, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { V3RunBody } from './scheduler/run.jsx';
import { useBrainRun } from './useRun.js';
import { PlanCanvas } from './history/canvas.jsx';
import { Inspector } from './history/inspector.jsx';
import { COLORS, PHASES, STATES, statusOf } from './history/model.js';
import { TimeText } from '../../ui/timeText.jsx';
function LegacyPlanBody({ run, onNotice }) {
  const [mode, setMode] = useState('execution'); const [stepId, setStepId] = useState(null); const [instanceId, setInstanceId] = useState(null);
  const select = (step) => { setStepId(step); setInstanceId(run?.groups?.find((group) => group.id === step)?.current_instance || run?.instances?.find((instance) => instance.step_id === step)?.id || null); };
  if (!run.plan) return <Empty description={run.phase === 'failed' ? '规划未完成' : '正在生成完整计划…'} />;
  const steps = run.plan.plan.schema_version === 2 ? run.plan.plan.instances : run.plan.plan.steps || [];
  return <><div className="brain-workspace-toolbar"><Space><Tag>历史 v2 运行</Tag><Typography.Text type="secondary">{run.total_instances || steps.length} 个实例</Typography.Text></Space><Button onClick={() => setMode(mode === 'execution' ? 'ontology' : 'execution')}>{mode === 'execution' ? '本体关系' : '执行状态'}</Button></div>
    <div className="brain-workspace"><aside className="brain-step-list">{steps.map((step) => { const group = run.groups?.find((item) => item.id === step.id); const status = statusOf(group); return <button className={stepId === step.id ? 'selected' : ''} key={step.id} onClick={() => select(step.id)}><small>{step.action.kind}</small><strong>{step.description || step.label}</strong><span>{STATES[status]}</span></button>; })}</aside><PlanCanvas plan={run.plan.plan} groups={run.groups} mode={mode} selected={stepId} onSelect={select} /><Inspector run={run} stepId={stepId} instanceId={instanceId} onInstance={setInstanceId} onNotice={onNotice} /></div>
  </>;
}

export function BrainRunBody({ id, onNotice, header = null }) {
  const { run, events, error, connection, refresh } = useBrainRun(id);
  return <div className="brain-run">{header}{error && <Alert type="error" showIcon title={error} action={<Button onClick={() => refresh()?.catch(() => {})}>重试</Button>} />}{!run ? <Spin /> : <>{!run.schema_version || run.schema_version < 3 ? <><div className="brain-run-header"><div><Typography.Title level={4}>{run.objective}</Typography.Title><Space wrap><Tag color={COLORS[run.phase]}>{PHASES[run.phase] || run.phase}</Tag><TimeText ts={run.updated_at} /></Space></div></div><Alert type="info" showIcon title="历史运行只读" /><LegacyPlanBody run={run} onNotice={onNotice} /></> : <V3RunBody view={run} id={id} onNotice={onNotice} events={events} connection={connection} refresh={refresh} />}</>}</div>;
}

export function BrainRunView({ id, onBack, onNotice }) {
  useEffect(() => { const params = new URLSearchParams(location.search); params.set('brain_run', id); history.replaceState(null, '', `${location.pathname}?${params}${location.hash}`); }, [id]);
  return <BrainRunBody id={id} onNotice={onNotice} header={<Space wrap><Button onClick={onBack}>返回工作台</Button><Breadcrumb items={[{ title: '大脑调度' }, { title: id }]} /></Space>} />;
}
