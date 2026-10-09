import { Alert, Collapse, Space, Tabs, Tag, Typography } from 'antd';
import { Markdown } from '../../../project/markdown.jsx';
import { TimeText } from '../../../ui/timeText.jsx';
import { InlineFields } from '../fields.jsx';
import { clarificationMembers } from './model.js';
import { TeamRecord } from './record.jsx';

function Summary({ record }) {
  return <>
    {typeof record.aligned === 'boolean' && <Tag color={record.aligned ? 'green' : 'orange'}>{record.aligned ? '已对齐' : '待澄清'}</Tag>}
    <div className="team-discussion-text"><Markdown text={record.summary} /></div>
    {!!record.ambiguities?.length && <div className="team-ambiguities">
      <Typography.Text strong>待澄清问题</Typography.Text>
      <ul>{record.ambiguities.map((item, index) => <li key={`${item.node_id}-${index}`}><Tag>{item.node_id}</Tag>{item.question}</li>)}</ul>
    </div>}
  </>;
}

function Statements({ id, round, step, captain, participants, questions = [] }) {
  const prefix = `team.turn.${round.number}.sub.${step}`;
  return <div className="team-discussion-statements">
    {participants.map((member) => <article className="team-statement" key={member}>
      <header><strong>{member}</strong><Tag color={member === captain ? 'gold' : undefined}>{member === captain ? '队长发言' : '成员发言'}</Tag></header>
      {questions.filter((item) => item.node_id === member).map((item, index) => <p className="team-followup" key={index}>待澄清：{item.question}</p>)}
      <TeamRecord id={id} field={`${prefix}.result.${member}`} label={`${member} 的发言`}>{(record) => <>
        {record.created_at ? <TimeText ts={record.created_at} /> : null}
        {record.ok === false && <Alert type="error" showIcon title="本次发言未成功" description={record.error || '成员未返回有效结果'} />}
        {record.answer && <div className="team-discussion-text"><Markdown text={record.answer} /></div>}
      </>}</TeamRecord>
    </article>)}
    {!participants.length && <Typography.Text type="secondary">本次没有需要补充发言的成员</Typography.Text>}
  </div>;
}

function DiscussionStep({ id, round, step, captain }) {
  return <div className="team-discussion-statements">
    {step === 0 ? <Statements id={id} round={round} step={step} captain={captain} participants={round.participants} />
      : <TeamRecord id={id} field={`team.turn.${round.number}.sub.${step - 1}.summary`} label="上次待澄清问题">{(previous) =>
        <Statements id={id} round={round} step={step} captain={captain} participants={clarificationMembers(previous)} questions={previous.ambiguities} />
      }</TeamRecord>}
    <article className="team-statement team-captain-summary">
      <header><strong>队长小结</strong><Tag color="gold">{captain || '队长'}</Tag></header>
      <TeamRecord id={id} field={`team.turn.${round.number}.sub.${step}.summary`} label="队长小结">{(record) => <Summary record={record} />}</TeamRecord>
    </article>
  </div>;
}

export function TeamRound({ id, round, captain }) {
  return <div>
    <Space wrap>{round.participants.map((member) => <Tag key={member}>{member}</Tag>)}</Space>
    <Collapse size="small" className="team-round-plan" items={[{
      key: 'plan', label: '本轮讨论安排', children: <TeamRecord id={id} field={round.plan} label="本轮讨论安排">{(record) => <>
        <Markdown text={record.question} />
        {record.rationale && <><Typography.Text type="secondary">选择这个问题的原因</Typography.Text><Markdown text={record.rationale} /></>}
      </>}</TeamRecord>,
    }]} />
    {round.steps > 0 ? <Tabs key={round.number} defaultActiveKey={String(round.steps - 1)} destroyOnHidden items={Array.from({ length: round.steps }, (_, step) => ({
      key: String(step), label: step === 0 ? '首次讨论' : `补充澄清 ${step}`,
      children: <DiscussionStep id={id} round={round} step={step} captain={captain} />,
    }))} /> : <Typography.Paragraph type="secondary">本轮发言和小结尚未就绪</Typography.Paragraph>}
    <InlineFields id={id} value={round.omitted} />
  </div>;
}
