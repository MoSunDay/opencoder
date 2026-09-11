import { Alert, Breadcrumb, Button, Collapse, Empty, Input, Segmented, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPost } from '../../api.js';
import { useBrainRun } from './useRun.js';
import { PlanCanvas } from './canvas.jsx';
import { Inspector } from './inspector.jsx';
import { COLORS, PHASES, STATES, statusOf, terminal } from './model.js';
import { TimeText } from '../../ui/timeText.jsx';

function InputRequests({ requests, onSubmit }) {
  const [values, setValues] = useState({});
  return Object.values(requests || {}).filter((r) => !r.answered).map((request) => <div key={request.name} className="brain-input-request"><Typography.Text strong>{request.name}</Typography.Text><Typography.Paragraph>{request.description}</Typography.Paragraph>
    <Space.Compact style={{ width: '100%' }}><Input value={values[request.name] || ''} onChange={(e) => setValues({ ...values, [request.name]: e.target.value })} placeholder={request.schema.type === 'string' ? '输入内容' : `输入 ${request.schema.type} JSON 值`} /><Button onClick={() => onSubmit(request.name, values[request.name] || '', request.schema.type)}>提交输入</Button></Space.Compact>
  </div>);
}
function Timeline({ id, liveEvents }) {
  const [history, setHistory] = useState([]); const [cursor, setCursor] = useState(0); const [more, setMore] = useState(true); const [error, setError] = useState(''); const [historical, setHistorical] = useState(false);
  const load = async () => { try { const page = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/events-page?after=${cursor}`); setHistory(page.events.map((e) => ({ seq: e.seq, event: e.kind, data: e.payload, ts: e.ts }))); setCursor(page.events.at(-1)?.seq || cursor); setMore(page.more); setHistorical(true); } catch (e) { setError(e.message); } };
  const events = historical ? history : liveEvents;
  return <><Space><Typography.Text strong>{historical ? '历史事件' : '实时事件'}</Typography.Text><Button size="small" disabled={historical && !more} onClick={load}>{historical ? '下一页' : '从头查看'}</Button>{historical && <Button size="small" onClick={() => { setHistorical(false); setCursor(0); setMore(true); }}>回到实时</Button>}</Space>
    {error && <Alert type="error" title={error} />}<div className="brain-events">{events.map((event) => <details key={event.seq}><summary><span>#{event.seq}</span><Tag>{event.event}</Tag><span>{event.data?.detail?.reason || event.data?.detail?.parent?.instance_id || ''}</span></summary><pre className="brain-json">{JSON.stringify(event.data, null, 2)}</pre></details>)}</div></>;
}
export function BrainRunView({ id, onBack, onNotice }) {
  const { run, events, error, connection, refresh } = useBrainRun(id); const [mode, setMode] = useState('execution'); const [stepId, setStepId] = useState(null); const [instanceId, setInstanceId] = useState(null); const [commandError, setCommandError] = useState(''); const [busy, setBusy] = useState(false);
  const select = (step) => { setStepId(step); setInstanceId(run?.instances.find((i) => i.step_id === step)?.id || null); };
  useEffect(() => { const params = new URLSearchParams(location.search); params.set('brain_run', id); history.replaceState(null, '', `${location.pathname}?${params}${location.hash}`); return () => {}; }, [id]);
  const command = async (action) => { setBusy(true); setCommandError(''); try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action }); await refresh(); } catch (e) { setCommandError(e.message); } finally { setBusy(false); } };
  const input = async (name, value, type) => { try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/inputs`, { name, value: type === 'string' ? value : JSON.parse(value) }); await refresh(); } catch (e) { setCommandError(e.message); } };
  return <div className="brain-run"><Space wrap><Button onClick={onBack}>返回工作台</Button><Breadcrumb items={[{ title: '大脑调度' }, { title: id }, ...(stepId ? [{ title: stepId }] : [])]} /></Space>
    {(error || commandError) && <Alert type="error" showIcon title={commandError || error} action={<Button onClick={() => refresh()?.catch(() => {})}>重试</Button>} />}
    {!run ? <Spin /> : <><div className="brain-run-header"><div><Typography.Title level={4}>{run.objective}</Typography.Title><Space wrap><Tag color={COLORS[run.phase]}>{PHASES[run.phase]}</Tag><Tag>{run.plan ? `${run.plan.id} · v${run.plan.version}` : '尚未生成计划'}</Tag><Typography.Text type="secondary">大脑：{['running', 'planning'].includes(run.phase) && run.handled_revision < run.revision ? '有事件待处理' : terminal(run.phase) ? '已结束' : '等待事件'}</Typography.Text><Typography.Text type="secondary">第 {run.activation} 次激活 · {connection === 'live' ? '实时连接' : connection}</Typography.Text><TimeText ts={run.updated_at} /></Space></div>
      <Space><Button disabled={busy || terminal(run.phase) || run.phase === 'cancelling'} onClick={() => command(run.phase === 'paused' ? 'resume' : 'pause')}>{run.phase === 'paused' ? '继续执行' : '暂停派发'}</Button><Button danger disabled={busy || terminal(run.phase) || run.phase === 'cancelling'} onClick={() => command('cancel')}>取消全部</Button></Space></div>
      {run.error && <Alert type="error" showIcon title="运行失败" description={run.error} />}
      <InputRequests requests={run.input_requests} onSubmit={input} />
      {!run.plan ? <Empty description={run.phase === 'failed' ? '规划未完成' : '正在生成完整计划…'} /> : <><div className="brain-workspace-toolbar"><Segmented value={mode} onChange={setMode} options={[{ value: 'execution', label: '执行状态' }, { value: 'ontology', label: '本体关系' }]} /><Typography.Text type="secondary">{run.total_instances} 个实例 · 运行固定在 v{run.plan.version}</Typography.Text></div>
      <div className="brain-workspace"><aside className="brain-step-list">{run.plan.plan.steps.map((step) => { const group = run.groups.find((g) => g.id === step.id); const status = statusOf(group); return <button className={stepId === step.id ? 'selected' : ''} key={step.id} onClick={() => select(step.id)}><small>{step.action.kind}{step.foreach ? ' · 批量' : ''}</small><strong>{step.label}</strong><span>{STATES[status]}{group?.total > 1 ? ` · ${group.counts.succeeded || 0}/${group.total}` : ''}</span></button>; })}</aside>
        <PlanCanvas plan={run.plan.plan} groups={run.groups} mode={mode} selected={stepId} onSelect={select} />
        <Inspector run={run} stepId={stepId} instanceId={instanceId} onInstance={setInstanceId} onNotice={onNotice} />
      </div></>}
      <Collapse defaultActiveKey={['timeline']} items={[{ key: 'timeline', label: '调度事件与因果记录', children: <Timeline id={id} liveEvents={events} /> }, { key: 'deliverables', label: '交付物与验收结果', children: <pre className="brain-json">{JSON.stringify(run.deliverables, null, 2)}</pre> }]} />
    </>}
  </div>;
}
