import { EditButton } from '../../../ui/permissions.jsx';
import { Alert, Button, Drawer, Input, InputNumber, Space, Tag, Typography } from 'antd';
import { FullscreenExitOutlined, FullscreenOutlined } from '@ant-design/icons';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { apiPost } from '../../../api.js';
import { ExecutionView } from '../../../fleet/detail.jsx';
import { MilestoneCanvas } from './canvas.jsx';
import { visits } from './model.js';
import { RunDetails } from './runDetails.jsx';
import { convertPlan } from '../scheduler/model.js';
import { LAYERED_COLORS, LAYERED_PHASES } from '../layered/model.js';
import { latestExecutions, selectExecution } from './runtime/model.js';
import { LayerDetails } from './runtime/layerDetails.jsx';
import { useRunViewport } from './runtime/viewport.js';
import './runtime/style.css';

export function MilestoneRunBody({ view, id, refresh, onNotice }) {
  const [executionId, setExecutionId] = useState(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [guidanceOpen, setGuidanceOpen] = useState(false);
  const scrollRef = useRef(null);
  const positions = useRef({});
  const submitting = useRef(false);
  const [layerId, setLayerId] = useState(null);
  const [selection, setSelection] = useState(null);
  const [fullscreen, setFullscreen] = useState(false);
  const viewport = useRunViewport(fullscreen);
  useEffect(() => {
    if (!fullscreen) return undefined;
    const escape = (event) => { if (event.key === 'Escape' && !detailsOpen) setFullscreen(false); };
    window.addEventListener('keydown', escape);
    return () => window.removeEventListener('keydown', escape);
  }, [fullscreen, detailsOpen]);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const [budget, setBudget] = useState(null);
  const [humanInput, setHumanInput] = useState('');
  const inputBytes = new TextEncoder().encode(humanInput.trim()).length;
  const { run, plan } = view;
  const displayPlan = (plan.schema_version || view.schema_version) === 7 ? plan : convertPlan({ ...plan, schema_version: plan.schema_version || view.schema_version });
  const history = visits(view);
  const visit = history.find((entry) => entry.activation === run.activation) || history[history.length - 1];
  const execution = (view.operations || []).find((op) => op.execution_id === executionId);
  const terminal = ['completed', 'failed', 'cancelled'].includes(run.phase);
  const readOnly = view.schema_version !== 7;
  const route = executionId || layerId || 'overview';
  const rememberPosition = () => {
    if (detailsOpen && scrollRef.current) positions.current[route] = scrollRef.current.scrollTop;
  };
  const showOverview = () => { rememberPosition(); setLayerId(null); setExecutionId(null); setDetailsOpen(true); };
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (element && detailsOpen) element.scrollTop = positions.current[route] || 0;
  }, [route, detailsOpen]);
  const command = async (action, input = {}) => {
    if (submitting.current || readOnly) return false;
    submitting.current = true; setBusy(true); setError('');
    try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action, input }); await refresh(); }
    catch (e) { setError(e.message); } finally { submitting.current = false; setBusy(false); }
  };
  const latest = latestExecutions(view.operations || []);
  const statuses = Object.fromEntries(plan.nodes.map((node) => {
    const operation = latest.find((op) => op.node_id === node.node_id);
    const label = !operation ? '尚未派发' : !['done', 'error', 'cancelled'].includes(operation.status) ? '执行中' : operation.status === 'done' ? '执行结束' : operation.status === 'error' ? '执行失败' : '已取消';
    return [node.node_id, label];
  }));
  const layerStatuses = Object.fromEntries(displayPlan.layers.map((layer, index) => [layer.layer_id,
    index < run.valid_layers ? '已达标' : !terminal && visit?.layer === index + 1 ? '当前执行' : '待达标']));
  const openExecution = (executionIdToOpen) => { if (executionIdToOpen) { rememberPosition(); setExecutionId(executionIdToOpen); setDetailsOpen(true); } };
  const selectLayerExecution = (nextId) => { rememberPosition(); setExecutionId(nextId); };
  const select = (next) => {
    const target = selectExecution(displayPlan, view.operations || [], next);
    rememberPosition();
    setSelection(next); setLayerId(target.layerId); setExecutionId(target.executionId); setDetailsOpen(true);
  };
  const sendInput = async (message) => {
    if (!message.trim()) return false;
    if (new TextEncoder().encode(message.trim()).length > 4096) { setError('人工输入不能超过 4096 字节'); return false; }
    if (submitting.current || readOnly) return false;
    submitting.current = true; setBusy(true); setError('');
    try {
      await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/inputs`, { text: message.trim() });
      try { await refresh(); } catch (e) { setError(`输入已记录，刷新失败：${e.message}`); }
      return true;
    } catch (e) { setError(e.message); return false; } finally { submitting.current = false; setBusy(false); }
  };
  const submitInput = async () => { if (await sendInput(humanInput)) setHumanInput(''); };
  return <section ref={viewport.ref} style={viewport.style} className={`brain-run-workspace${fullscreen ? ' is-fullscreen' : ''}`} aria-label="大脑执行画布">
    <div className="brain-run-status">
      <div className="brain-run-title"><strong title={plan.title}>{plan.title}</strong><Space wrap><Tag>第 {run.round} / {run.max_rounds} 轮</Tag><Tag>{run.valid_layers || 0} / {displayPlan.layers.length} 层已达标</Tag><Tag color={run.phase === 'completed' ? 'success' : undefined}>{LAYERED_PHASES[run.phase] || run.phase}</Tag></Space></div>
      <Space wrap>
        {!terminal && !readOnly && <><Button disabled={busy} onClick={() => command(['paused', 'blocked'].includes(run.phase) ? 'resume' : 'pause')}>{['paused', 'blocked'].includes(run.phase) ? '继续调度' : '暂停调度'}</Button>
          <EditButton danger disabled={busy} onClick={() => command('cancel')}>取消运行</EditButton></>}
        <Button onClick={showOverview}>查看详情</Button>
        <Button aria-label={fullscreen ? '退出全屏' : '全屏画布'} icon={fullscreen ? <FullscreenExitOutlined /> : <FullscreenOutlined />} onClick={() => setFullscreen(!fullscreen)}>{fullscreen ? '退出全屏' : '全屏画布'}</Button>
      </Space>
    </div>
    {!detailsOpen && (error || run.error) && <Alert type="error" showIcon title={error || run.error} />}
    <div className="brain-milestone-preview"><MilestoneCanvas plan={displayPlan} capabilities={view.capabilities || []} statuses={statuses} layerStatuses={layerStatuses}
      direction="horizontal" selection={selection} onSelect={select} /></div>
    <Drawer rootClassName="brain-details-drawer" placement="right" title={<div className="brain-detail-title">
      <Typography.Title level={5}>{plan.title || '计划运行详情'}</Typography.Title>
      <Typography.Text className="brain-detail-id" type="secondary" copyable>{id}</Typography.Text>
      <Space wrap><Tag color={LAYERED_COLORS[run.phase]}>{LAYERED_PHASES[run.phase] || run.phase}</Tag><Tag>第 {run.round} / {run.max_rounds} 轮</Tag>{readOnly && <Tag>历史版本 · 只读</Tag>}</Space>
    </div>} open={detailsOpen} onClose={() => { rememberPosition(); setDetailsOpen(false); setExecutionId(null); }} size="min(1200px, 100vw)">
      <div className="brain-detail-scroll" ref={scrollRef}>
        {error && <Alert type="error" showIcon title={error} />}
        <div hidden={!!layerId || !!execution}>
          {!terminal && !readOnly && <Button className="brain-child-heading" onClick={() => setGuidanceOpen(!guidanceOpen)} aria-expanded={guidanceOpen}>补充信息</Button>}
          {guidanceOpen && !terminal && !readOnly && <div className="brain-guidance-panel"><Space orientation="vertical" style={{ width: '100%' }}>
            <Input.TextArea aria-label="大脑人工输入" value={humanInput} onChange={(e) => setHumanInput(e.target.value)} rows={3}
              placeholder="补充信息或调整调度要求；信息将作为最新事件交给大脑" />
            <span className={inputBytes > 4096 ? 'brain-input-count over-limit' : 'brain-input-count'}>{inputBytes} / 4096 字节</span>
            <EditButton aria-label="发送大脑输入" type="primary" loading={busy} disabled={!humanInput.trim() || inputBytes > 4096} onClick={submitInput}>发送给大脑</EditButton>
          </Space></div>}
          {!readOnly && ['paused', 'blocked'].includes(run.phase) && <Space wrap style={{ marginBottom: 16 }}>
            <InputNumber aria-label="新的轮次预算" min={run.round + 1} max={32} precision={0} value={budget} onChange={setBudget} />
            <EditButton disabled={busy || !budget} onClick={() => command('set_round_budget', { max_rounds: budget })}>调整预算</EditButton>
          </Space>}
          <RunDetails id={id} view={view} plan={displayPlan} history={history} onExecution={openExecution} visible={detailsOpen && !layerId && !execution} />
        </div>
        {detailsOpen && layerId ? <>
          <Button className="brain-child-heading" onClick={showOverview}>返回运行详情</Button>
          <Typography.Title level={5}>第 {displayPlan.layers.findIndex((layer) => layer.layer_id === layerId) + 1} 层 · {displayPlan.layers.find((layer) => layer.layer_id === layerId)?.title}</Typography.Title>
          <LayerDetails layerId={layerId} view={view} plan={displayPlan} executionId={executionId} onExecution={selectLayerExecution}
            terminal={terminal || readOnly} onGuidance={sendInput} onNotice={onNotice} />
        </> : detailsOpen && execution ? <>
          <div className="brain-child-heading"><Button onClick={showOverview}>返回运行详情</Button>
            <Typography.Title level={5}>{plan.nodes.find((node) => node.node_id === execution.node_id)?.title || execution.node_id}</Typography.Title>
            <Space wrap><Tag>第 {execution.round} 轮 · 第 {execution.layer} 层</Tag><Typography.Text copyable code>{execution.execution_id}</Typography.Text></Space>
          </div>
          <ExecutionView key={execution.execution_id} executionRef={{ id: execution.execution_id, kind: execution.execution_kind }} presentation="brain"
            managed allowGuidance={!terminal && !readOnly} onGuidance={sendInput} onNotice={onNotice} />
        </> : null}
      </div>
    </Drawer>
  </section>;
}
