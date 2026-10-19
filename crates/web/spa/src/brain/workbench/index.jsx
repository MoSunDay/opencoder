import { Alert, Button, Drawer, Tabs } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { BrainPanel } from '../../brainPanel.jsx';
import { PageShell } from '../../shell/pageShell.jsx';
import { RunHistory } from './history/index.jsx';
import { useHistory } from './history/useHistory.js';
import { Launch } from './launch.jsx';
import { Plans } from './plans.jsx';
import { BrainRunView } from './run.jsx';
import { replaceLocation } from '../../fleet/navigation/history.js';
import './style.css';
export function BrainWorkbench({ onNotice }) {
  const [tab, setTab] = useState('workspace'); const [plans, setPlans] = useState([]); const [error, setError] = useState(''); const [runId, setRunId] = useState(() => new URLSearchParams(location.search).get('brain_run')); const [launch, setLaunch] = useState(false); const [initialPlan, setInitialPlan] = useState(null);
  const history = useHistory();
  const scroll = useRef(0);
  const openRun = (id) => { scroll.current = document.querySelector('.fleet-content')?.scrollTop || 0; setRunId(id); };
  useEffect(() => { if (!runId) { const frame = requestAnimationFrame(() => { const pane = document.querySelector('.fleet-content'); if (pane) pane.scrollTop = scroll.current; }); return () => cancelAnimationFrame(frame); } }, [runId]);
  const [catalog, setCatalog] = useState({ capabilities: null, loading: true, error: '' });
  const catalogRequest = useRef(null);
  const capabilities = catalog.capabilities || [];
  const reloadCapabilities = useCallback(async () => {
    catalogRequest.current?.abort();
    const request = new AbortController();
    catalogRequest.current = request;
    setCatalog((previous) => ({ ...previous, loading: true, error: '' }));
    try {
      const result = await apiGet('/api/brain/library', { signal: request.signal });
      if (!request.signal.aborted) setCatalog({ capabilities: result.capabilities || [], selectionIds: result.selection_ids, loading: false, error: '' });
    } catch (e) {
      if (!request.signal.aborted) setCatalog((previous) => ({ ...previous, loading: false, error: e.message }));
    }
  }, []);
  useEffect(() => () => catalogRequest.current?.abort(), []);
  const reload = useCallback(async () => { try { const [p] = await Promise.all([apiGet('/api/brain/plan-defs'), reloadCapabilities()]); setPlans(p.plans); setError(''); } catch (e) { setError(e.message); } }, [reloadCapabilities]);
  useEffect(() => { reload(); }, [reload, tab]);
  const executePlan = (plan) => { setInitialPlan(plan); setLaunch(true); };
  const back = () => { setRunId(null); const query = new URLSearchParams(location.search); query.delete('brain_run'); replaceLocation(`${location.pathname}${query.size ? `?${query}` : ''}${location.hash}`); };
  return <PageShell page="brain">{runId ? <BrainRunView key={runId} id={runId} onBack={back} onNotice={onNotice} /> : <>
    {error && <Alert type="error" showIcon title={error} action={<Button onClick={reload}>重试</Button>} />}
    {catalog.error && tab !== 'capabilities' && <Alert type="error" showIcon title={`获取执行绑定失败: ${catalog.error}`} action={<Button onClick={reloadCapabilities}>重试</Button>} />}
    <Tabs activeKey={tab} onChange={setTab} items={[
      { key: 'workspace', label: '运行历史', children: <RunHistory history={history} onOpen={openRun} /> },
      { key: 'plans', label: '计划库', children: <Plans plans={plans} capabilities={capabilities} selectionIds={catalog.selectionIds} reload={reload} onRun={executePlan} /> },
      { key: 'capabilities', label: '能力库', children: <BrainPanel onNotice={onNotice} executionCatalog={catalog.capabilities} catalogLoading={catalog.loading} catalogError={catalog.error} onRefreshCatalog={reloadCapabilities} /> },
    ]} />
  </>}
  <Drawer destroyOnHidden open={launch} onClose={() => setLaunch(false)} title="新建大脑运行" size={600}><Launch key={String(initialPlan)} capabilities={capabilities} initialPlan={initialPlan} onCreated={(id) => { setLaunch(false); history.load(0); openRun(id); }} /></Drawer>
  </PageShell>;
}
