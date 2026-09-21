// run.jsx — the v4 workbench body: layered canvas, layer barrier progress,
// layer decisions and the v4 event journal. v3 runs never reach this module.
import { Alert, Button, Collapse, Progress, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../../api.js';
import { KIND_LABELS } from '../../../fleet/model.js';
import { TimeText } from '../../../ui/timeText.jsx';
import { LayerCanvas } from './canvas.jsx';
import { LayeredEvents } from './events.jsx';
import { LayerRounds } from './rounds.jsx';
import { LAYERED_COLORS, LAYERED_PHASES, barrier, layeredPhase, planOf, terminalPhase } from './model.js';
import './style.css';

/// Connection badge text. A v4 run has no stream in the locked contract, so
/// the hook also polls every 3s: a failed stream means "polling", not "broken".
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

export function LayeredRunBody({ view, id, connection, refresh }) {
  const [busy, setBusy] = useState(false); const [commandError, setCommandError] = useState(''); const [selected, setSelected] = useState(null);
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
          <Tag color="purple">v4 分层能力画布</Tag>
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
    <LayerCanvas view={view} selected={selected} onSelect={setSelected} />
    <section className="brain-rounds">
      <Typography.Title level={5}>分层决策与执行</Typography.Title>
      <LayerRounds id={id} view={view} />
    </section>
    <Collapse items={[
      { key: 'events', label: `分层事件（${(view.events || []).length}）`, children: <LayeredEvents view={view} /> },
      { key: 'capabilities', label: `本次运行关联能力（${(view.capabilities || []).length}）`, children: <CapabilityList capabilities={view.capabilities} /> },
    ]} />
  </>;
}
