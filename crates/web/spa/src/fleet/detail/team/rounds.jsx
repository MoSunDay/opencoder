import { Alert, Button, Collapse, Empty, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../../api.js';
import { turnView } from './model.js';
import { TeamRound } from './round.jsx';
import { LiveTeamRound } from './live.jsx';

export function TeamRounds({ id, topic, captain, running, unfinished }) {
  const [cursors, setCursors] = useState([null]);
  const [index, setIndex] = useState(0);
  const [page, setPage] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [expanded, setExpanded] = useState(null);
  const cursor = cursors[index];
  useEffect(() => {
    if (cursor === null) { setBusy(false); setError(''); return undefined; }
    const controller = new AbortController();
    let timer;
    setBusy(true); setError('');
    const load = async () => {
      try {
        const result = await apiGet(`/api/executions/${encodeURIComponent(id)}/team-turns?after_turn=${encodeURIComponent(cursor)}`, { signal: controller.signal });
        if (!controller.signal.aborted) { setPage({ cursor, turns: result.turns || [], next: result.next_turn }); setError(''); }
      } catch (failure) { if (!controller.signal.aborted) setError(failure.message || '读取讨论轮次失败'); }
      finally { if (!controller.signal.aborted) { setBusy(false); if (running) timer = setTimeout(load, 3000); } }
    };
    load();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [id, cursor, retry, running]);
  const current = index === 0
    ? { turns: topic.turns || [], next: topic.turns_page?.next_turn }
    : page?.cursor === cursor ? page : { turns: [], next: null };
  const rounds = current.turns.map(turnView);
  const next = current.next !== null && current.next !== undefined;
  const pending = busy || (cursor !== null && page?.cursor !== cursor && !error);
  const navigate = (target) => { setIndex(target); setExpanded(null); };
  const advance = () => {
    setCursors(cursors.slice(0, index + 1).concat(current.next));
    navigate(index + 1);
  };
  return <section className="team-discussion-rounds">
    <div className="team-discussion-heading"><Typography.Title level={5}>讨论过程</Typography.Title>
      <Typography.Text type="secondary">{rounds.length ? `${index === 0 && !next ? '已记录' : '本页'} ${rounds.length} 轮` : ''}</Typography.Text>
    </div>
    {!next && !pending && !error && (running || unfinished || ['error', 'cancelled'].includes(topic.finish_reason)) && <LiveTeamRound
      key={`${id}-${rounds.at(-1)?.number || 0}`} id={id} number={(rounds.at(-1)?.number || 0) + 1} captain={captain} running={running} />}
    {error && <Alert type="error" showIcon title="讨论轮次读取失败" description={error}
      action={<Button size="small" onClick={() => setRetry((value) => value + 1)}>重试</Button>} />}
    {pending && <Spin size="small" />}
    {!pending && !error && !rounds.length && !running && <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无已完成的讨论轮次" />}
    <Collapse accordion destroyOnHidden activeKey={expanded ?? (rounds.length ? [String(rounds.at(-1).number)] : [])}
      onChange={setExpanded} items={rounds.map((round) => ({
        key: String(round.number),
        label: <div className="team-round-heading"><strong>第 {round.number} 轮</strong><span title={round.question}>{round.question}</span></div>,
        extra: typeof round.aligned === 'boolean' && <Tag color={round.aligned ? 'green' : 'orange'}>{round.aligned ? '已对齐' : '待澄清'}</Tag>,
        children: <TeamRound key={round.number} id={id} round={round} captain={captain} />,
      }))} />
    {(index > 0 || next) && <Space className="team-round-pagination" wrap>
      <Button size="small" disabled={pending || index === 0} onClick={() => navigate(index - 1)}>上一页轮次</Button>
      <Typography.Text type="secondary">第 {index + 1} 页</Typography.Text>
      <Button size="small" disabled={pending || !next} onClick={advance}>下一页轮次</Button>
    </Space>}
  </section>;
}
