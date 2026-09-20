import { Button, Space, Tag, Typography } from 'antd';
import { V3_COLORS, V3_PHASES, currentRound, phaseOf, roundLabel } from './v3Model.js';
import './style.css';

export function SummaryCanvas({ view, onCapabilities }) {
  const phase = phaseOf(view); const round = currentRound(view); const active = !!view.run;
  const stages = [
    ['目标与输入', view.objective || '明确目标和交付物', false],
    ['大脑调度', active ? V3_PHASES[phase] || phase : '从所选能力中决定下一步', ['ready', 'deciding', 'paused', 'blocked'].includes(phase) && active],
    ['本轮能力执行', active ? roundLabel(round) : '可并行调度多项能力', phase === 'waiting'],
    ['结果判断', '依据执行结果判断是否继续', phase === 'deciding'],
    ['完成', active && ['completed', 'failed', 'cancelled'].includes(phase) ? V3_PHASES[phase] : '交付目标与验证依据', phase === 'completed'],
  ];
  const caps = view.capabilities || []; const count = view.capability_ids?.length || caps.length;
  return <section className="brain-core-flow" aria-label="大脑调度总览画布">
    <div className="brain-core-stages">{stages.map(([title, value, selected], index) => <div className="brain-core-stage" key={title}>
      {index > 0 && <span aria-hidden="true" className="brain-summary-arrow">→</span>}
      <div className={`brain-summary-node${selected ? ' brain-stage-active' : ''}`}><Typography.Text type="secondary">{title}</Typography.Text><Typography.Text strong title={value}>{value}</Typography.Text></div>
    </div>)}</div>
    <div className="brain-core-loop" aria-label="继续下一轮，返回大脑调度">↖ 继续下一轮：结果判断 → 大脑调度</div>
    <div className="brain-flow-capabilities"><Space wrap><Typography.Text strong>关联能力库 → 大脑调度</Typography.Text><Tag>{count} 项能力</Tag>
      {onCapabilities && <Button size="small" onClick={onCapabilities}>查看关联能力</Button>}</Space>
      <Space wrap className="brain-capability-preview">{caps.slice(0, 5).map((cap) => <Tag title={cap.target || cap.capability_id || cap.id} key={cap.capability_id || cap.id}>{cap.kind} · {cap.target || cap.capability_id || cap.id}</Tag>)}{caps.length > 5 && <Tag>+{caps.length - 5}</Tag>}</Space>
    </div>
    {active && <Space wrap><Tag color={V3_COLORS[phase]}>{V3_PHASES[phase] || phase}</Tag>{view.plan && <Typography.Text type="secondary">计划 {view.plan.id} · v{view.plan.version}</Typography.Text>}</Space>}
  </section>;
}
