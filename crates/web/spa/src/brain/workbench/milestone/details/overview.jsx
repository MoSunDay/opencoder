import { Alert, Descriptions, Empty, Space, Tag, Typography } from 'antd';
import { ReadableText } from '../../../../ui/runContent/value.jsx';
import { TimeText } from '../../../../ui/timeText.jsx';
import { LAYERED_PHASES, LAYERED_COLORS } from '../../layered/model.js';

export function Overview({ view, plan }) {
  const { run } = view;
  const current = [...(view.events || [])].reverse().find((event) => event.event_type === 'layer_started' && event.activation === run.activation);
  return <div className="run-content-stack">
    {run.error && <Alert type={run.phase === 'blocked' ? 'warning' : 'error'} showIcon title={run.phase === 'blocked' ? '运行已阻塞' : '运行异常'} description={<ReadableText text={run.error} />} />}
    <section className="brain-result-summary"><Typography.Title level={5}>交付摘要</Typography.Title>
      {run.summary ? <ReadableText text={run.summary} markdown /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={['completed', 'failed', 'cancelled'].includes(run.phase) ? '未记录交付摘要' : '尚未生成最终结果'} />}</section>
    <Descriptions size="small" column={{ xs: 1, sm: 2 }} items={[
      { key: 'phase', label: '运行状态', children: <Tag color={LAYERED_COLORS[run.phase]}>{LAYERED_PHASES[run.phase] || run.phase}</Tag> },
      { key: 'round', label: '轮次', children: `第 ${run.round} / ${run.max_rounds} 轮` },
      { key: 'layer', label: '最近派发阶段', children: current ? `第 ${current.layer} 层 · ${plan.layers[current.layer - 1]?.title || ''}` : '尚未派发' },
      { key: 'valid', label: '当前有效达标', children: `${run.valid_layers || 0} / ${plan.layers.length} 层` },
      { key: 'created', label: '创建时间', children: <TimeText ts={run.created_at} /> },
      { key: 'updated', label: '更新时间', children: <TimeText ts={run.updated_at} /> },
    ]} />
    <section><Typography.Title level={5}>计划目标</Typography.Title><ReadableText text={plan.objective || '未记录'} markdown /></section>
    {run.reflection && <section><Space><Typography.Title level={5}>当前整改说明</Typography.Title></Space><ReadableText text={run.reflection} markdown /></section>}
  </div>;
}
