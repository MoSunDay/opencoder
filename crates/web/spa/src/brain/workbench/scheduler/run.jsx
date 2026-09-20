import { Alert, Button, Collapse, Descriptions, Drawer, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../../api.js';
import { TimeText } from '../../../ui/timeText.jsx';
import { SummaryCanvas } from '../summaryCanvas.jsx';
import { V3_COLORS, V3_PHASES, currentRound, phaseOf, roundsOf, terminalV3 } from '../v3Model.js';
import { RoundsPanel } from './rounds.jsx';
import { ExecutionDrawer } from './detail.jsx';
import { Timeline } from './events.jsx';

export function V3RunBody({ view, id, onNotice, events, connection, refresh }) {
  const [selected, setSelected] = useState(null); const [showCapabilities, setShowCapabilities] = useState(false); const [commandError, setCommandError] = useState(''); const [busy, setBusy] = useState(false);
  const phase = phaseOf(view); const run = view.run;
  const operation = roundsOf(view).flatMap((round) => round.operations).find((operation) => operation.execution_id === selected);
  const select = (operation) => setSelected(operation.execution_id);
  const command = async (action) => { setBusy(true); setCommandError(''); try { await apiPost(`/api/brain/runs/${encodeURIComponent(id)}/commands`, { action }); await refresh(); } catch (error) { setCommandError(error.message); } finally { setBusy(false); } };
  return <>
    {(commandError || view.error) && <Alert type="error" showIcon title={commandError || view.error} />}
    {run.error && <Alert type="error" showIcon title="运行阻塞或失败" description={run.error} />}
    <div className="brain-run-header"><div><Typography.Title level={4} ellipsis={{ rows: 2, expandable: 'collapsible', symbol: (expanded) => expanded ? '收起目标' : '展开目标' }}>{view.objective || run.run_id}</Typography.Title><Space wrap><Tag color={V3_COLORS[phase]}>{V3_PHASES[phase] || phase}</Tag><Tag>第 {currentRound(view)} 轮</Tag><Typography.Text type="secondary">{['open', 'live'].includes(connection) ? '实时连接' : connection === 'closed' ? '运行已结束' : '正在同步'}</Typography.Text><TimeText ts={run.updated_at} /></Space></div>
      <Space><Button disabled={busy || terminalV3(view)} onClick={() => command(phase === 'paused' ? 'resume' : 'pause')}>{phase === 'paused' ? '继续调度' : '暂停调度'}</Button><Button danger disabled={busy || terminalV3(view)} onClick={() => command('cancel')}>取消调度</Button></Space></div>
    <SummaryCanvas view={view} onCapabilities={() => setShowCapabilities(true)} />
    <Drawer title="本次运行关联能力" open={showCapabilities} onClose={() => setShowCapabilities(false)} size={560}>
      {(view.capabilities || []).map((capability) => <section className="brain-operation-card" key={capability.capability_id}><Space><Tag>{capability.kind}</Tag><Typography.Text strong>{capability.target}</Typography.Text></Space><Typography.Text code>{capability.capability_id}</Typography.Text><Typography.Paragraph>输入：{capability.input_desc || '历史记录未保存描述'}</Typography.Paragraph><Typography.Paragraph>输出：{capability.output_desc || '历史记录未保存描述'}</Typography.Paragraph></section>)}
    </Drawer>
    <section className="brain-rounds"><Typography.Title level={5}>调度轮次与能力执行</Typography.Title><RoundsPanel view={view} onOpen={select} /></section>
    <Collapse items={[{ key: 'timeline', label: '调度诊断与事件', children: <Timeline id={id} liveEvents={events} /> }, { key: 'inputs', label: '工程输入名称', children: <Descriptions size="small" column={1} items={(view.input_names || []).map((name) => ({ key: name, label: name, children: '已保存在本次运行' }))} /> }]} />
    <ExecutionDrawer view={view} operation={operation} onSelect={select} onClose={() => setSelected(null)} onNotice={onNotice} />
  </>;
}
