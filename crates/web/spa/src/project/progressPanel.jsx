// progressPanel.jsx — 菜单页「进展」: the project module's live heartbeat.
// Three surfaces over ONE useOverview snapshot (same adaptive poll as 项目):
// ① 里程碑进度卡 — per-milestone done/total Progress with goal context and
// the milestone's own status tag; ② 进行中 TODO — flattenTodos filtered by
// the todosTab busy 口径 (running, or an execution pending/running/cancelling)
// with elapsed TimeText; ③ 最近项目执行 — kind=project executions page whose
// rows open ExecutionDetail. Derivations (milestoneProgress / milestoneCards
// / liveTodos) are pure + exported so panels and tests share one definition.

import { Card, Col, Empty, Progress, Row, Space, Spin, Table, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { ExecutionDetail } from '../fleet/detail.jsx';
import { executionPagePath } from '../fleet/model.js';
import { PageShell } from '../shell/pageShell.jsx';
import { StatusTag } from '../ui/statusTag.jsx';
import { TimeText } from '../ui/timeText.jsx';
import { MilestoneStatusTag } from './labels.jsx';
import { flattenTodos } from './todosTab.jsx';
import { useOverview } from './useOverview.js';

const { Text } = Typography;
const RECENT_EXECUTIONS = 10;

/// milestoneProgress(milestone) → { done, total, percent } — the done/total
/// rollup behind every progress bar. total=0 yields percent 0 (callers render
/// the gray 无 TODO instead of a bar).
export function milestoneProgress(milestone) {
  const todos = (milestone && milestone.todos) || [];
  const done = todos.filter((t) => t.status === 'done').length;
  const total = todos.length;
  return { done, total, percent: total ? Math.round((done / total) * 100) : 0 };
}

/// milestoneCards(overview) → one row per milestone carrying its goal title
/// (cards show the parent goal) plus the pure progress numbers.
export function milestoneCards(overview) {
  const goals = (overview && overview.goals) || [];
  return goals.flatMap((g) => (g.milestones || []).map((m) => ({
    key: m.id,
    goal_title: g.title,
    milestone: m,
    ...milestoneProgress(m),
  })));
}

/// isTodoLive(t) — todosTab 的 busy 口径 verbatim: a running todo, or any
/// todo whose node execution is pending/running/cancelling.
export function isTodoLive(t) {
  return t.status === 'running'
    || ['pending', 'running', 'cancelling'].includes(t.execution?.status);
}

/// liveTodos(overview) → the 进行中 list rows (milestone + backlog, with
/// milestone/goal context from flattenTodos).
export function liveTodos(overview) {
  return flattenTodos(overview).filter(isTodoLive);
}

/// 所属 cell: goal / milestone, or 未分组 for backlog rows. Shared with the
/// Owner 视角 attention table (one wording for the same context).
export function ownerLabel(row) {
  return row.goal_title && row.milestone_title
    ? `${row.goal_title} / ${row.milestone_title}`
    : '未分组';
}

/// Elapsed anchor: the execution's started_at, falling back to the todo's
/// updated_at; null when both are missing (cell renders '—').
function elapsedTs(row) {
  return (row.execution && row.execution.started_at) || row.updated_at || null;
}

/// One milestone progress card: goal title (secondary) + milestone title &
/// status tag, then the done/total bar (or the gray 无 TODO placeholder).
function MilestoneCard({ card }) {
  return (
    <Col key={card.key} xs={24} sm={12} lg={8} xl={6}>
      <Card size="small">
        <Text type="secondary" style={{ fontSize: 12 }}>{card.goal_title}</Text>
        <div style={{ marginTop: 4, marginBottom: 8 }}>
          <Space size={8} wrap>
            <Text strong>{card.milestone.title}</Text>
            <MilestoneStatusTag status={card.milestone.status} />
          </Space>
        </div>
        {card.total > 0 ? (
          <Space size={8} align="center" style={{ width: '100%' }}>
            <Progress percent={card.percent} size="small" style={{ flex: 1, minWidth: 80, marginBottom: 0 }} />
            <Text type="secondary">{card.done}/{card.total}</Text>
          </Space>
        ) : (
          <Text type="secondary">无 TODO</Text>
        )}
      </Card>
    </Col>
  );
}

export function ProgressPanel({ onNotice }) {
  const { overview, loading, refresh } = useOverview({ onNotice });
  const [executions, setExecutions] = useState([]);
  const [detail, setDetail] = useState(null);

  // Errors surface through the latest onNotice without re-arming the fetch.
  const noticeRef = useRef(onNotice);
  useEffect(() => {
    noticeRef.current = onNotice;
  }, [onNotice]);

  // The execution page rides the overview cadence: every overview refresh
  // (adaptive 3s/8s poll or an explicit `refresh` after a write) pulls the
  // latest kind=project executions page alongside.
  useEffect(() => {
    let live = true;
    apiGet(executionPagePath('project', null, RECENT_EXECUTIONS))
      .then((page) => {
        if (live) {
          setExecutions((page && page.executions) || []);
        }
        return null;
      })
      .catch((e) => {
        const notify = noticeRef.current;
        if (live && notify) {
          notify('获取项目执行失败: ' + (e && e.message));
        }
      });
    return () => {
      live = false;
    };
  }, [overview, refresh]);

  const goals = (overview && overview.goals) || [];
  const cards = milestoneCards(overview);
  const live = liveTodos(overview);

  const liveColumns = [
    { title: '标题', dataIndex: 'title', render: (v) => <Text strong>{v}</Text> },
    { title: '所属', key: 'owner', render: (_, r) => ownerLabel(r) },
    {
      title: '已耗时',
      key: 'elapsed',
      render: (_, r) => {
        const ts = elapsedTs(r);
        return ts ? <TimeText ts={ts} /> : <Text type="secondary">—</Text>;
      },
    },
  ];

  const execColumns = [
    {
      title: 'ID',
      dataIndex: 'id',
      render: (v) => <span style={{ fontFamily: 'var(--oc-mono, monospace)' }}>{v}</span>,
    },
    { title: '状态', dataIndex: 'status', render: (v) => <StatusTag status={v} /> },
    { title: '节点', dataIndex: 'node_id', render: (v) => <span style={{ fontFamily: 'var(--oc-mono, monospace)' }}>{v || '—'}</span> },
    { title: '创建时间', dataIndex: 'created_at', render: (v) => <TimeText ts={v} /> },
  ];

  return (
    <PageShell page="progress">
      {goals.length === 0 ? (
        <Empty description="暂无项目目标" style={{ margin: '48px 0' }} />
      ) : (
        <Spin spinning={loading}>
          <Row gutter={[12, 12]}>
            {cards.map((card) => <MilestoneCard key={card.key} card={card} />)}
          </Row>
          <Card size="small" title="进行中 TODO" style={{ marginTop: 12 }}>
            <Table
              rowKey="id"
              size="small"
              columns={liveColumns}
              dataSource={live}
              pagination={false}
              locale={{ emptyText: '当前没有进行中的 TODO' }}
            />
          </Card>
          <Card size="small" title="最近项目执行" style={{ marginTop: 12 }}>
            <Table
              rowKey="id"
              size="small"
              columns={execColumns}
              dataSource={executions}
              pagination={false}
              locale={{ emptyText: '还没有项目执行' }}
              onRow={(row) => ({ onClick: () => setDetail(row), style: { cursor: 'pointer' } })}
            />
          </Card>
        </Spin>
      )}
      {detail ? (
        <ExecutionDetail
          id={detail.id}
          summary={detail}
          onClose={() => setDetail(null)}
          onNotice={onNotice}
        />
      ) : null}
    </PageShell>
  );
}
