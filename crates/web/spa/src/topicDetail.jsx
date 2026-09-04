// topicDetail.jsx — 话题详情页: two-column shell mirroring chatSidebar's
// layout. Left: turn timeline (antd Timeline; green dot = aligned). Right:
// the selected turn's plan card, per-sub-turn member results (Collapse —
// answer full text, error line when ok=false, 对齐追答 marked by kind) and
// the summary + ambiguities chain. The final summary card mounts once the
// topic is finished. Executing topics keep the same 3s poll as the list
// tabs; finished topics stop polling.

import { Button, Card, Collapse, Space, Spin, Tag, Timeline, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from './api.js';
import { closeTopicDetail, useStore } from './store.js';
import {
  ambiguityText,
  fmtTime,
  resultLabel,
  subTurnCount,
  topicStatusView,
  turnAligned,
} from './teamItems.js';

const { Text, Paragraph } = Typography;
const POLL_MS = 3000;

function PlanCard({ plan }) {
  const p = plan || {};
  return (
    <Card size="small" title="汇报计划 (plan)">
      <Paragraph style={{ marginBottom: 8 }}>{p.question || '-'}</Paragraph>
      <div style={{ marginBottom: 8 }}>
        {(p.participants || []).map((id) => (
          <Tag key={id} color="blue">{id}</Tag>
        ))}
      </div>
      <Text type="secondary" style={{ whiteSpace: 'pre-wrap' }}>{p.rationale || ''}</Text>
    </Card>
  );
}

function ResultList({ results }) {
  const items = (results || []).map((r, i) => ({
    key: (r && r.node_id) + ':' + i,
    label: resultLabel(r),
    children: (
      <div>
        {r && r.ok === false ? (
          <Paragraph type="danger" style={{ whiteSpace: 'pre-wrap', marginBottom: 4 }}>
            {'error: ' + (r.error || '未知错误')}
          </Paragraph>
        ) : null}
        <Paragraph style={{ whiteSpace: 'pre-wrap', marginBottom: 0 }}>{(r && r.answer) || '-'}</Paragraph>
      </div>
    ),
  }));
  return items.length > 0 ? <Collapse size="small" items={items} /> : <Text type="secondary">暂无汇报</Text>;
}

function SummaryBlock({ summary }) {
  const s = summary || {};
  return (
    <div style={{ border: '1px solid #f0f0f0', borderRadius: 8, padding: '6px 12px', marginTop: 8 }}>
      <Space size={8} wrap>
        <Text strong style={{ fontSize: 12 }}>子轮小结</Text>
        <Tag color={s.aligned ? 'success' : 'warning'}>{s.aligned ? '已对齐' : '未对齐'}</Tag>
      </Space>
      <Paragraph style={{ marginTop: 6, marginBottom: 6, whiteSpace: 'pre-wrap' }}>{s.summary || '-'}</Paragraph>
      {(s.ambiguities || []).length > 0 ? (
        <div>
          <Text type="secondary" style={{ fontSize: 12 }}>对齐链 (ambiguities)</Text>
          {(s.ambiguities || []).map((a, i) => (
            <Paragraph key={i} style={{ marginBottom: 2, fontSize: 12 }}>{ambiguityText(a)}</Paragraph>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function SubTurnBlock({ sub }) {
  return (
    <Card size="small" title={'子轮 ' + ((sub && sub.sub_turn) || '-')}>
      <ResultList results={sub && sub.results} />
      <SummaryBlock summary={sub && sub.summary} />
    </Card>
  );
}

export function TopicDetailPanel({ onNotice }) {
  const { topicDetail } = useStore();
  const teamName = topicDetail && topicDetail.teamName;
  const topicId = topicDetail && topicDetail.topicId;
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(false);
  const [turnSel, setTurnSel] = useState(null);
  const alive = useRef(true);

  const load = useCallback(async (silent) => {
    if (!teamName || !topicId) {
      return;
    }
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet(
        '/api/teams/' + encodeURIComponent(teamName) + '/topics/' + encodeURIComponent(topicId),
      );
      if (!alive.current) {
        return;
      }
      setData(j || null);
    } catch (e) {
      if (!silent && alive.current && onNotice) {
        onNotice('获取话题失败: ' + (e && e.message));
      }
    } finally {
      if (alive.current && !silent) {
        setLoading(false);
      }
    }
  }, [onNotice, teamName, topicId]);

  useEffect(() => {
    alive.current = true;
    load(false);
    return () => {
      alive.current = false;
    };
  }, [load]);

  const topic = (data && data.topic) || null;
  const turns = (data && data.turns) || [];
  const executing = !!topic && topic.status === 'executing';

  // Poll only while executing — a finished topic is frozen server-side.
  useEffect(() => {
    if (!executing) {
      return undefined;
    }
    const t = setInterval(() => load(true), POLL_MS);
    return () => clearInterval(t);
  }, [executing, load]);

  // Selection follows the data: default to the latest turn, follow the old
  // selection while it still exists (new turns arrive during polling).
  useEffect(() => {
    if (turns.length > 0 && !turns.some((t) => t.turn === turnSel)) {
      setTurnSel(turns[turns.length - 1].turn);
    }
  }, [turns, turnSel]);

  const sel = turns.find((t) => t.turn === turnSel) || null;
  const status = topicStatusView(topic);
  const timelineItems = turns.map((t) => ({
    key: t.turn,
    color: turnAligned(t) ? 'green' : 'gray',
    children: (
      <div
        onClick={() => setTurnSel(t.turn)}
        style={{ cursor: 'pointer', padding: '2px 0' }}
      >
        <Space size={6} wrap>
          <Text strong={t.turn === turnSel}>{'Turn ' + t.turn}</Text>
          <Tag color={turnAligned(t) ? 'success' : 'warning'} style={{ marginRight: 0 }}>
            {turnAligned(t) ? '已对齐' : '未对齐'}
          </Tag>
          <Text type="secondary" style={{ fontSize: 12 }}>{'子轮 ' + subTurnCount(t)}</Text>
        </Space>
        <Paragraph style={{ marginBottom: 0, fontSize: 12 }} ellipsis>
          {(t.plan && t.plan.question) || t.question || ''}
        </Paragraph>
      </div>
    ),
  }));

  if (!topicDetail) {
    return null;
  }

  return (
    <div>
      <div style={{ marginBottom: 12, display: 'flex', alignItems: 'center', gap: 12 }}>
        <Button size="small" onClick={closeTopicDetail}>← 返回话题列表</Button>
        <Text strong style={{ fontSize: 15 }}>{(topic && topic.title) || topicId}</Text>
        <Tag color={status.color}>{status.label}</Tag>
        <Tag>{(topic && topic.team_name) || teamName}</Tag>
        <Text type="secondary">{fmtTime(topic && topic.created_at)}</Text>
      </div>
      <Spin spinning={loading}>
        <div style={{ display: 'flex', gap: 16, alignItems: 'flex-start' }}>
          <div style={{ width: 300, flexShrink: 0, overflow: 'auto', maxHeight: '70vh', borderRight: '1px solid #f0f0f0', paddingRight: 12 }}>
            {timelineItems.length > 0 ? (
              <Timeline items={timelineItems} />
            ) : (
              <Text type="secondary">暂无轮次{executing ? '，等待首个汇报计划…' : ''}</Text>
            )}
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            {sel ? (
              <Space direction="vertical" size={12} style={{ width: '100%' }}>
                <PlanCard plan={sel.plan} />
                {(sel.sub_turns || []).map((sub) => (
                  <SubTurnBlock key={sub.sub_turn} sub={sub} />
                ))}
              </Space>
            ) : (
              <Text type="secondary">选择左侧轮次查看汇报详情</Text>
            )}
            {topic && topic.status === 'finished' && topic.final_summary ? (
              <Card size="small" title="最终总结" style={{ marginTop: 12 }}>
                <Paragraph style={{ whiteSpace: 'pre-wrap', marginBottom: 0 }}>{topic.final_summary}</Paragraph>
              </Card>
            ) : null}
          </div>
        </div>
      </Spin>
    </div>
  );
}
