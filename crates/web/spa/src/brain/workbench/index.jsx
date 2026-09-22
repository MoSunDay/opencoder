import { Alert, Button, Drawer, Empty, Space, Table, Tabs, Tag } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { BrainPanel } from '../../brainPanel.jsx';
import { PageShell } from '../../shell/pageShell.jsx';
import { StatusTag } from '../../ui/statusTag.jsx';
import { Launch } from './launch.jsx';
import { Plans } from './plans.jsx';
import { BrainRunView } from './run.jsx';
import './style.css';
export function BrainWorkbench({ onNotice }) {
  const [tab, setTab] = useState('workspace'); const [runCursor, setRunCursor] = useState(null); const [plans, setPlans] = useState([]); const [runs, setRuns] = useState([]); const [capabilities, setCapabilities] = useState([]); const [error, setError] = useState(''); const [runId, setRunId] = useState(() => new URLSearchParams(location.search).get('brain_run')); const [launch, setLaunch] = useState(false); const [initialPlan, setInitialPlan] = useState(null);
  const reload = useCallback(async () => { try { const [p, r, c] = await Promise.all([apiGet('/api/brain/plan-defs'), apiGet('/api/brain/runs'), apiGet('/api/brain/library')]); setPlans(p.plans); setRuns(r.runs); setRunCursor(r.next_cursor || null); setCapabilities(c.capabilities); setError(''); } catch (e) { setError(e.message); } }, []);
  useEffect(() => { reload(); }, [reload, tab]);
  const olderRuns = async () => { try { const page = await apiGet(`/api/brain/runs?cursor_created_at=${runCursor.created_at}&cursor_id=${encodeURIComponent(runCursor.id)}`); setRuns(page.runs); setRunCursor(page.next_cursor || null); } catch (e) { setError(e.message); } };
  const executePlan = (plan) => { setInitialPlan(plan); setLaunch(true); };
  const back = () => { setRunId(null); const query = new URLSearchParams(location.search); query.delete('brain_run'); history.replaceState(null, '', `${location.pathname}${query.size ? `?${query}` : ''}${location.hash}`); reload(); };
  return <PageShell page="brain">{runId ? <BrainRunView key={runId} id={runId} onBack={back} onNotice={onNotice} /> : <>
    {error && <Alert type="error" showIcon title={error} action={<Button onClick={reload}>重试</Button>} />}
    <Tabs activeKey={tab} onChange={setTab} items={[
      { key: 'workspace', label: '工作台', children: <><Space style={{ marginBottom: 16 }}><Button type="primary" onClick={() => { setTab('plans'); }}>创建并执行计划</Button><Button onClick={reload}>刷新运行</Button><Button disabled={!runCursor} onClick={olderRuns}>更早运行</Button></Space><Table rowKey="id" dataSource={runs} pagination={{ pageSize: 10 }} columns={[
        { title: '运行', dataIndex: 'id', render: (id) => <Button type="link" onClick={() => setRunId(id)}>{id}</Button> }, { title: '节点', dataIndex: 'node_id' }, { title: '执行状态', dataIndex: 'status', render: (status) => status === 'idle' ? <Tag color="blue">等待事件</Tag> : <StatusTag status={status} /> },
      ]} locale={{ emptyText: <Empty description="从目标开始，让能力组成可观察的执行计划" /> }} /></> },
      { key: 'plans', label: '计划库', children: <Plans plans={plans} capabilities={capabilities} reload={reload} onRun={executePlan} /> },
      { key: 'capabilities', label: '能力库', children: <BrainPanel onNotice={onNotice} /> },
    ]} />
  </>}
  <Drawer destroyOnHidden open={launch} onClose={() => setLaunch(false)} title="新建大脑运行" size={600}><Launch key={String(initialPlan)} capabilities={capabilities} initialPlan={initialPlan} onCreated={(id) => { setLaunch(false); setRunId(id); }} /></Drawer>
  </PageShell>;
}
