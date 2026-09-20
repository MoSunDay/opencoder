import { memo } from 'react';
import { Space, Tag, Typography } from 'antd';
import { V3_COLORS, V3_PHASES, currentRound, phaseOf, roundLabel } from './v3Model.js';
import './style.css';

const SummaryNode = memo(function SummaryNode({ title, value, color, detail }) {
  return <div className="brain-summary-node">
    <Typography.Text type="secondary">{title}</Typography.Text>
    <Typography.Paragraph strong ellipsis={{ rows: 3, tooltip: value }} style={{ marginBottom: 0 }}>{value}</Typography.Paragraph>
    {color && <Tag color={color}>{detail || value}</Tag>}
  </div>;
});

export function SummaryCanvas({ view }) {
  const phase = phaseOf(view);
  const round = currentRound(view);
  const run = view?.run || {};
  return <section className="brain-summary-canvas" aria-label="大脑调度总览画布">
    <SummaryNode title="目标 / 计划" value={view?.objective || run.run_id || '未命名任务'} detail={view?.schema_version === 3 ? 'v3 事件调度' : '历史运行'} />
    <span className="brain-summary-arrow" aria-hidden="true">→</span>
    <SummaryNode title="当前阶段" value={V3_PHASES[phase] || phase} color={V3_COLORS[phase]} />
    <span className="brain-summary-arrow" aria-hidden="true">→</span>
    <SummaryNode title="调度轮次" value={roundLabel(round)} detail={`${view?.rounds?.length || 0} 轮有记录`} />
    <span className="brain-summary-arrow" aria-hidden="true">→</span>
    <SummaryNode title="结果" value={V3_PHASES[phase] || phase} color={V3_COLORS[phase]} detail={run.error || (phase === 'waiting' ? '等待本轮能力终态' : undefined)} />
    <Space className="brain-summary-meta" size={8} wrap>
      <Typography.Text type="secondary">运行 ID：{run.run_id || '—'}</Typography.Text>
      <Typography.Text type="secondary">事件序号：{run.last_event_seq ?? '—'}</Typography.Text>
    </Space>
  </section>;
}
