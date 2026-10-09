import { Alert, Button, Spin, Tag, Typography } from 'antd';
import { PayloadWindows } from '../fields.jsx';
import { TeamRound } from './round.jsx';
import { useRoundProgress } from './progress.js';

export function LiveTeamRound({ id, number, captain, running }) {
  const state = useRoundProgress({ id, number, running });
  const waiting = number === 1 ? '队长正在安排首轮讨论' : `已完成 ${number - 1} 轮，队长正在总结或安排下一轮`;
  return <section className="team-live-round" aria-label="当前讨论进展">
    {state.error && <Alert type="error" showIcon title="当前讨论读取失败" description={state.error}
      action={<Button onClick={state.retry}>重试</Button>} />}
    {state.busy && <Spin size="small" />}
    {!state.plan && !state.large && !state.error && running && <Alert type="info" showIcon title={waiting}
      description="执行已受理。讨论安排、成员发言和队长小结生成后会自动显示，无需重复启动。" />}
    {state.large && <PayloadWindows id={id} marker={{ field: `team.turn.${number}.plan`, read_via: 'detail_field', omitted: true }} label="分段查看当前讨论安排" />}
    {state.plan && <>
      <div className="team-discussion-heading"><Typography.Title level={5}>第 {number} 轮</Typography.Title>
        <Tag color={running ? 'processing' : 'default'}>{!running ? '未完成轮次' : state.phase === 'closing' ? '队长正在收尾' : state.step > 0 ? '补充澄清中' : '成员讨论中'}</Tag>
      </div>
      <Typography.Paragraph>{state.plan.question}</Typography.Paragraph>
      <TeamRound id={id} captain={captain} live running={running} round={{ number, question: state.plan.question,
        participants: state.plan.participants || [], steps: state.step + 1, plan: `team.turn.${number}.plan` }} />
    </>}
  </section>;
}
