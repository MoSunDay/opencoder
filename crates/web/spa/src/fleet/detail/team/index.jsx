import { Alert, Descriptions, Space, Tag, Typography } from 'antd';
import { Markdown } from '../../../project/markdown.jsx';
import { InlineFields } from '../fields.jsx';
import { topicNotice } from './model.js';
import { TeamRounds } from './rounds.jsx';
import './style.css';

// Reused by Team history, the execution drawer and embedded execution views.
export function TeamExecutionProcess({ id, detail }) {
  const team = detail.definition || {};
  const topic = detail.topic || {};
  const captain = team.captain || topic.captain?.name || topic.captain?.node_id;
  const notice = topicNotice(topic, detail.execution?.status);
  const running = notice?.[1] === '讨论进行中';
  const requirement = topic.requirement || detail.request?.input?.prompt;
  const final = topic.final_summary;
  return <section className="team-discussion" aria-label="Team 讨论">
    <Typography.Title level={5}>Team 讨论</Typography.Title>
    <Descriptions size="small" column={{ xs: 1, sm: 2 }} items={[
      { key: 'name', label: 'Team', children: team.name || topic.team_name || '—' },
      { key: 'captain', label: '队长', children: captain || '—' },
      { key: 'members', label: '成员', span: { xs: 1, sm: 2 }, children: <Space wrap>{(team.members || []).map((member) =>
        <Tag key={member.agent} className="team-member-tag" color={member.agent === captain ? 'gold' : undefined}
          title={[member.agent, ...(member.capabilities || [])].join(' · ')}>{member.agent}</Tag>,
      )}</Space> },
    ]} />
    {notice && <Alert type={notice[0]} showIcon title={notice[1]} description={notice[2]} />}
    {typeof final === 'string' && final.trim() && <section className="team-discussion-conclusion">
      <Typography.Title level={5}>{notice?.[0] === 'success' ? '最终结论' : '阶段小结'}</Typography.Title>
      <div className="team-discussion-text"><Markdown text={final} /></div>
    </section>}
    {typeof requirement === 'string' && requirement.trim() && <section className="team-discussion-goal">
      <Typography.Title level={5}>讨论目标</Typography.Title>
      <div className="team-discussion-text"><Markdown text={requirement} /></div>
    </section>}
    <InlineFields id={id} value={{ requirement, final }} />
    <TeamRounds key={id} id={id} topic={topic} captain={captain} running={running}
      unfinished={['error', 'cancelled', 'interrupted'].includes(detail.execution?.status)} />
  </section>;
}
