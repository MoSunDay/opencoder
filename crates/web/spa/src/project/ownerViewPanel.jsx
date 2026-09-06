// ownerViewPanel.jsx — 菜单页「Owner 视角」: the one-glance project health
// page. Same useOverview snapshot as 项目/进展. Three surfaces: ① 待人工介入
// (置顶, rendered ONLY when it has content) — failed TODOs with a 处理 button
// into the panel-owned TodoDrawer, plus 阻塞提示 rows (planned todos whose
// updated_at has been quiet past BLOCKED_AFTER_MS = 24h); ② 按目标分组 rollup
// — one Card per goal with the pure goalHealth Badge and per-milestone
// done/total mini bars; ③ the backlog count strip. All derivations (goalTodos
// / goalHealth / isStalePlanned / failedTodos / blockedTodos / attentionRows)
// are pure + exported for tests.

import { Badge, Button, Card, Col, Empty, Progress, Row, Space, Spin, Table, Typography } from 'antd';
import { useState } from 'react';
import { PageShell } from '../shell/pageShell.jsx';
import { TimeText } from '../ui/timeText.jsx';
import { milestoneProgress, ownerLabel } from './progressPanel.jsx';
import { TodoDrawer } from './todoDrawer.jsx';
import { flattenTodos } from './todosTab.jsx';
import { useOverview } from './useOverview.js';

const { Text } = Typography;

/// A planned todo untouched for longer than this is flagged as 阻塞 (24h).
export const BLOCKED_AFTER_MS = 24 * 60 * 60 * 1000;

/// goalTodos(goal) → the goal's milestone todos (backlog is NOT part of a
/// goal). Pure.
export function goalTodos(goal) {
  return ((goal && goal.milestones) || []).flatMap((m) => m.todos || []);
}

/// goalHealth(goal) → { status, label } for the antd Badge preset:
/// 全 done=success 已完成；有 failed=error 需介入；有 running=processing
/// 进行中；否则 default 规划中. Checked in that priority order.
export function goalHealth(goal) {
  const todos = goalTodos(goal);
  if (todos.length > 0 && todos.every((t) => t.status === 'done')) {
    return { status: 'success', label: '已完成' };
  }
  if (todos.some((t) => t.status === 'failed')) {
    return { status: 'error', label: '需介入' };
  }
  if (todos.some((t) => t.status === 'running')) {
    return { status: 'processing', label: '进行中' };
  }
  return { status: 'default', label: '规划中' };
}

/// isStalePlanned(todo, now) — planned AND updated_at older than
/// BLOCKED_AFTER_MS. A missing updated_at can never be judged stale.
export function isStalePlanned(todo, now = Date.now()) {
  if (!todo || todo.status !== 'planned' || !todo.updated_at) {
    return false;
  }
  return now - todo.updated_at > BLOCKED_AFTER_MS;
}

/// failedTodos(overview) → every failed todo row with milestone/goal context.
export function failedTodos(overview) {
  return flattenTodos(overview).filter((t) => t.status === 'failed');
}

/// blockedTodos(overview, now) → planned rows quiet past the 24h threshold.
export function blockedTodos(overview, now = Date.now()) {
  return flattenTodos(overview).filter((t) => isStalePlanned(t, now));
}

/// attentionRows(overview, now) → the 待人工介入 agenda: failed first, then
/// blocked; each row tagged `attention: 'failed' | 'blocked'` for rendering.
export function attentionRows(overview, now = Date.now()) {
  return [
    ...failedTodos(overview).map((t) => ({ ...t, attention: 'failed' })),
    ...blockedTodos(overview, now).map((t) => ({ ...t, attention: 'blocked' })),
  ];
}

/// One goal rollup card: title + health Badge, then per-milestone mini bars
/// (done/total text; 无 TODO placeholder when empty).
function GoalCard({ goal }) {
  const health = goalHealth(goal);
  const milestones = goal.milestones || [];
  return (
    <Col xs={24} lg={12}>
      <Card
        size="small"
        title={<Text strong>{goal.title}</Text>}
        extra={<Badge status={health.status} text={health.label} />}
      >
        {milestones.length === 0 ? (
          <Text type="secondary">暂无里程碑</Text>
        ) : milestones.map((m) => {
          const p = milestoneProgress(m);
          return (
            <div key={m.id} style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
              <Text style={{ flex: 1 }} ellipsis>{m.title}</Text>
              {p.total > 0 ? (
                <>
                  <Progress percent={p.percent} size="small" showInfo={false} style={{ width: 120, marginBottom: 0 }} />
                  <Text type="secondary" style={{ minWidth: 32, textAlign: 'right' }}>{p.done}/{p.total}</Text>
                </>
              ) : (
                <Text type="secondary">无 TODO</Text>
              )}
            </div>
          );
        })}
      </Card>
    </Col>
  );
}

export function OwnerViewPanel({ onNotice }) {
  const { overview, loading, refresh } = useOverview({ onNotice });
  const [todoId, setTodoId] = useState(null); // open TODO drawer

  const goals = (overview && overview.goals) || [];
  const backlog = (overview && overview.backlog) || [];
  const attention = attentionRows(overview);

  const attentionColumns = [
    { title: '标题', dataIndex: 'title', render: (v) => <Text strong>{v}</Text> },
    { title: '所属', key: 'owner', render: (_, r) => ownerLabel(r) },
    {
      title: '说明',
      key: 'note',
      render: (_, r) => (r.attention === 'failed' ? (
        <Space size={4}>失败于 <TimeText ts={r.updated_at} /></Space>
      ) : (
        <Space size={4}>已规划长时间未执行（<TimeText ts={r.updated_at} /> 未动）</Space>
      )),
    },
    {
      title: '操作',
      key: 'ops',
      width: 90,
      render: (_, r) => (r.attention === 'failed' ? (
        <Button type="link" size="small" onClick={() => setTodoId(r.id)}>处理</Button>
      ) : (
        <Text type="secondary">—</Text>
      )),
    },
  ];

  return (
    <PageShell page="ownerview">
      {goals.length === 0 ? (
        <Empty description="暂无项目目标" style={{ margin: '48px 0' }} />
      ) : (
        <Spin spinning={loading}>
          {attention.length > 0 ? (
            <Card size="small" title="待人工介入" style={{ marginBottom: 12 }}>
              <Table
                rowKey={(r) => `${r.attention}:${r.id}`}
                size="small"
                columns={attentionColumns}
                dataSource={attention}
                pagination={false}
              />
            </Card>
          ) : null}
          <Row gutter={[12, 12]}>
            {goals.map((g) => <GoalCard key={g.id} goal={g} />)}
          </Row>
          <Card size="small" style={{ marginTop: 12 }}>
            <Text type="secondary">未分组 TODO（backlog）：</Text>
            <Text strong>{backlog.length}</Text>
            <Text type="secondary"> 条</Text>
          </Card>
        </Spin>
      )}
      <TodoDrawer
        key={todoId || 'closed'}
        todoId={todoId}
        overview={overview}
        refresh={refresh}
        onClose={() => setTodoId(null)}
        onNotice={onNotice}
      />
    </PageShell>
  );
}
