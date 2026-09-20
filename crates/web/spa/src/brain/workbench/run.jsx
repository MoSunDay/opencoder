import { Alert, Breadcrumb, Button, Collapse, Descriptions, Drawer, Empty, Input, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPost } from '../../api.js';
import { ExecutionView } from '../../fleet/detail.jsx';
import { useBrainRun } from './useRun.js';
import { PlanCanvas } from './canvas.jsx';
import { Inspector } from './inspector.jsx';
import { COLORS, PHASES, STATES, statusOf } from './model.js';
import { TimeText } from '../../ui/timeText.jsx';
import { SummaryCanvas } from './summaryCanvas.jsx';
import { V3_COLORS, V3_PHASES, V3_STATUS, capabilityFor, currentRound, normalizeEvent, phaseOf, roundsOf, roundLabel, terminalV3 } from './v3Model.js';

function InputRequests({ requests, onSubmit }) {
  const [values, setValues] = useState({});
  return Object.values(requests || {}).filter((request) => !request.answered).map((request) => <div key={request.name} className="brain-input-request"><Typography.Text strong>{request.name}</Typography.Text><Typography.Paragraph>{request.description}</Typography.Paragraph>
    <Space.Compact style={{ width: '100%' }}><Input value={values[request.name] || ''} onChange={(event) => setValues({ ...values, [request.name]: event.target.value })} placeholder={request.schema.type === 'string' ? '输入内容' : `输入 ${request.schema.type} JSON 值`} /><Button onClick={() => onSubmit(request.name, values[request.name] || '', request.schema.type)}>提交输入</Button></Space.Compact>
  </div>);
}

function Timeline({ id, liveEvents }) {
  const [history, setHistory] = useState([]); const [cursor, setCursor] = useState(0); const [more, setMore] = useState(true); const [error, setError] = useState(''); const [historical, setHistorical] = useState(false);
  const load = async () => { try { const page = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/events-page?after=${cursor}`); const events = (page.events || []).map(normalizeEvent).filter(Boolean); setHistory(events); setCursor(events.at(-1)?.seq || cursor); setMore(page.more); setHistorical(true); } catch (event) { setError(event.message); } };
  const events = historical ? history : liveEvents;
  return <><Space><Typography.Text strong>{historical ? '历史调度事件' : '实时调度事件'}</Typography.Text><Button size="small" disabled={historical && !more} onClick={load}>{historical ? '下一页' : '从头查看'}</Button>{historical && <Button size="small" onClick={() => { setHistorical(false); setCursor(0); setMore(true); }}>回到实时</Button>}</Space>
    {error && <Alert type="error" title={error} />}<div className="brain-events">{events.map((event) => <details key={event.seq}><summary><span>#{event.seq}</span><Tag>{event.event}</Tag><span>{event.data?.reason_summary || event.data?.execution_id || ''}</span></summary><pre className="brain-json">{JSON.stringify(event.data, null, 2)}</pre></details>)}</div></>;
}

function ExecutionDrawer({ operation, capability, onClose, onNotice }) {
  if (!operation) return null;
  const title = `${capability?.kind || operation.execution_kind} · ${capability?.target || capability?.capability_id || operation.capability_id}`;
  const summary = { id: operation.execution_id, kind: operation.execution_kind, status: operation.status === 'done' ? 'done' : operation.status === 'error' ? 'error' : operation.status === 'cancelled' ? 'cancelled' : 'running' };
  return <Drawer open title={<Space><span>{title}</span><Typography.Text code>{operation.execution_id}</Typography.Text></Space>} placement="right" size="78vw" onClose={onClose}>
    <ExecutionView executionRef={{ id: operation.execution_id, kind: operation.execution_kind }} summary={summary} onNotice={onNotice} managed />
  </Drawer>;
}

function OperationCard({ operation, capability, onOpen }) {
  const label = capability?.target || capability?.capability_id || operation.capability_id;
  return <article className="brain-operation-card">
    <Space wrap><Tag color={V3_COLORS[operation.status]}>{V3_STATUS[operation.status] || operation.status}</Tag><Tag>{capability?.kind || operation.execution_kind}</Tag><Typography.Text strong>{label}</Typography.Text></Space>
    <Typography.Text type="secondary">能力 ID：{operation.capability_id}{capability?.version ? ` · v${capability.version}` : ''}</Typography.Text>
    <Space wrap><Typography.Text type="secondary">来源序号：{operation.source_sequence ?? '—'}</Typography.Text>{operation.cancel_requested && <Tag color="orange">已请求取消</Tag>}<Button type="link" size="small" onClick={() => onOpen(operation)}>查看执行 {operation.execution_id}</Button></Space>
  </article>;
}

function RoundsPanel({ view, onOpen }) {
  const rounds = roundsOf(view); const current = currentRound(view);
  if (!rounds.length) return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未产生调度轮次" />;
  return <Collapse defaultActiveKey={[String(current)]} items={rounds.map((round) => ({
    key: String(round.round),
    label: <Space><Typography.Text strong>{roundLabel(round.round)}</Typography.Text><Tag color={V3_COLORS[round.status]}>{V3_PHASES[round.status] || round.status}</Tag><Typography.Text type="secondary">{round.operations.length} 个能力</Typography.Text></Space>,
    children: <div className="brain-operation-list">{round.operations.map((operation) => <OperationCard key={operation.operation_id} operation={operation} capability={capabilityFor(view, operation)} onOpen={onOpen} />)}</div>,
  }))} />;
}

function V3RunBody({ view, id, onNotice, events, connection, refresh }) {
  const [operation, setOperation] = useState(null); const [commandError, setCommandError] = useState(''); const [busy, setBusy] = useState(false);
  const phase = phaseOf(view); const run = view.run;
  const command = async (action) => { setBusy(true); setCommandError(''); try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action }); await refresh(); } catch (error) { setCommandError(error.message); } finally { setBusy(false); } };
  return <>
    {(commandError || view.error) && <Alert type="error" showIcon title={commandError || view.error} />}
    {run.error && <Alert type="error" showIcon title="运行阻塞或失败" description={run.error} />}
    <div className="brain-run-header"><div><Typography.Title level={4}>{view.objective || run.run_id}</Typography.Title><Space wrap><Tag color={V3_COLORS[phase]}>{V3_PHASES[phase] || phase}</Tag><Tag>第 {currentRound(view)} 轮</Tag><Typography.Text type="secondary">{connection === 'live' ? '实时连接' : connection}</Typography.Text><TimeText ts={run.updated_at} /></Space></div>
      <Space><Button disabled={busy || terminalV3(view)} onClick={() => command(phase === 'paused' ? 'resume' : 'pause')}>{phase === 'paused' ? '继续调度' : '暂停调度'}</Button><Button danger disabled={busy || terminalV3(view)} onClick={() => command('cancel')}>取消调度</Button></Space></div>
    <SummaryCanvas view={view} />
    <section className="brain-rounds"><Space className="brain-rounds-heading" wrap><Typography.Title level={5}>调度轮次与能力执行</Typography.Title><Typography.Text type="secondary">点击 execution ID 查看节点执行面板</Typography.Text></Space><RoundsPanel view={view} onOpen={setOperation} /></section>
    <Collapse defaultActiveKey={['timeline']} items={[{ key: 'timeline', label: '调度事件与索引记录', children: <Timeline id={id} liveEvents={events} /> }, { key: 'inputs', label: '工程输入名称', children: <Descriptions size="small" column={1} items={(view.input_names || []).map((name) => ({ key: name, label: name, children: '由根请求保存，执行详情按 execution ID 查询' }))} /> }]} />
    <ExecutionDrawer operation={operation} capability={operation && capabilityFor(view, operation)} onClose={() => setOperation(null)} onNotice={onNotice} />
  </>;
}

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
  const { run, events, error, connection, refresh } = useBrainRun(id); const [commandError, setCommandError] = useState('');
  const input = async (name, value, type) => { try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/inputs`, { name, value: type === 'string' ? value : JSON.parse(value) }); await refresh(); } catch (event) { setCommandError(event.message); } };
  return <div className="brain-run">{header}{(error || commandError) && <Alert type="error" showIcon title={commandError || error} action={<Button onClick={() => refresh()?.catch(() => {})}>重试</Button>} />}{!run ? <Spin /> : <>{!run.schema_version || run.schema_version < 3 ? <><div className="brain-run-header"><div><Typography.Title level={4}>{run.objective}</Typography.Title><Space wrap><Tag color={COLORS[run.phase]}>{PHASES[run.phase] || run.phase}</Tag><TimeText ts={run.updated_at} /></Space></div></div><InputRequests requests={run.input_requests} onSubmit={input} /><LegacyPlanBody run={run} onNotice={onNotice} /></> : <V3RunBody view={run} id={id} onNotice={onNotice} events={events} connection={connection} refresh={refresh} />}</>}</div>;
}

export function BrainRunView({ id, onBack, onNotice }) {
  useEffect(() => { const params = new URLSearchParams(location.search); params.set('brain_run', id); history.replaceState(null, '', `${location.pathname}?${params}${location.hash}`); }, [id]);
  return <BrainRunBody id={id} onNotice={onNotice} header={<Space wrap><Button onClick={onBack}>返回工作台</Button><Breadcrumb items={[{ title: '大脑调度' }, { title: id }]} /></Space>} />;
}
