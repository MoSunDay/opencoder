// project.jsx — 菜单页「项目」: 项目模块总控。Owns the /api/project/overview
// state + adaptive polling (3s while any todo is running, else 8s — interval
// re-arms when the busy flag flips, mirroring nodes.jsx), and renders four
// tabs: 总览 / 项目目标 / 里程碑 / TODO. `refresh` is a silent reload handed
// to every tab so writes converge into the single overview snapshot. The
// TODO drawer (详情/生成Plan 跳转) is owned here and keyed by todoId.

import { Card, Col, Row, Spin, Statistic, Tabs, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { GoalsTab } from './goalsTab.jsx';
import { MilestonesTab } from './milestonesTab.jsx';
import { TodoDrawer } from './todoDrawer.jsx';
import { TodosTab, flattenTodos } from './todosTab.jsx';

const { Text, Paragraph } = Typography;
const POLL_BUSY_MS = 3000;
const POLL_IDLE_MS = 8000;

function overviewBusy(overview) {
  return flattenTodos(overview).some((t) => t.status === 'running');
}

/// 总览 tab: counters + the workflow hint. Pure function of `overview`.
function OverviewTab({ overview }) {
  const goals = (overview && overview.goals) || [];
  const backlog = (overview && overview.backlog) || [];
  const milestones = goals.flatMap((g) => g.milestones || []);
  const todos = [...milestones.flatMap((m) => m.todos || []), ...backlog];
  const by = (s) => todos.filter((t) => t.status === s).length;
  const cards = [
    { title: '目标', value: goals.length, suffix: '个' },
    { title: '里程碑', value: milestones.filter((m) => m.status === 'done').length, suffix: ` / ${milestones.length} 完成` },
    { title: 'TODO 草稿', value: by('draft') },
    { title: 'TODO 已规划', value: by('planned') },
    { title: 'TODO 处理中', value: by('running') },
    { title: 'TODO 完成', value: by('done') },
    { title: 'TODO 失败', value: by('failed') },
    { title: '未分组 TODO', value: backlog.length },
  ];
  return (
    <Row gutter={[12, 12]}>
      {cards.map((c) => (
        <Col key={c.title} xs={12} sm={8} md={6}>
          <Card size="small"><Statistic title={c.title} value={c.value} suffix={c.suffix || ''} /></Card>
        </Col>
      ))}
      <Col span={24}>
        <Card size="small">
          <Paragraph style={{ marginBottom: 0 }}>
            <Text strong>工作流：</Text>
            <Text>
              草稿（粗略想法）→ 生成Plan（LLM 出结构化计划，版本留存）→ 执行（独立会话跑
              plan，输出与 session 归档）→ 每次运行都有版本可回看。目标 → 里程碑 → TODO
              三级组织，未分组 TODO 放在 backlog。
            </Text>
          </Paragraph>
        </Card>
      </Col>
    </Row>
  );
}

export function ProjectPanel({ onNotice }) {
  const [overview, setOverview] = useState(null);
  const [loading, setLoading] = useState(false);
  const [todoId, setTodoId] = useState(null); // open TODO drawer
  const timer = useRef(null);
  const alive = useRef(true);
  const busy = overviewBusy(overview);

  // Keep `load` identity STABLE regardless of the onNotice prop identity: a
  // caller passing a fresh inline arrow (tests, memo boundaries) must not
  // re-arm the mount effect into a fetch→setState→render loop.
  const noticeRef = useRef(onNotice);
  useEffect(() => {
    noticeRef.current = onNotice;
  }, [onNotice]);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/project/overview');
      if (alive.current) {
        setOverview(j || { goals: [], backlog: [] });
      }
    } catch (e) {
      if (!silent && alive.current) {
        const notify = noticeRef.current;
        if (notify) {
          notify('获取项目总览失败: ' + (e && e.message));
        }
      }
    } finally {
      if (alive.current && !silent) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    load(false);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [load]);

  // Adaptive poll: fast while a todo runs, slow otherwise; re-armed on flip.
  useEffect(() => {
    timer.current = setInterval(() => load(true), busy ? POLL_BUSY_MS : POLL_IDLE_MS);
    return () => clearInterval(timer.current);
  }, [busy, load]);

  const tabs = [
    { key: 'overview', label: '总览', children: <OverviewTab overview={overview} /> },
    { key: 'goals', label: '项目目标', children: <GoalsTab overview={overview} refresh={() => load(true)} onNotice={onNotice} /> },
    { key: 'milestones', label: '里程碑', children: <MilestonesTab overview={overview} refresh={() => load(true)} onNotice={onNotice} /> },
    { key: 'todos', label: 'TODO', children: <TodosTab overview={overview} refresh={() => load(true)} openTodo={setTodoId} onNotice={onNotice} /> },
  ];

  return (
    <div>
      <Spin spinning={loading}>
        <Tabs items={tabs} />
      </Spin>
      <TodoDrawer
        todoId={todoId}
        overview={overview}
        refresh={() => load(true)}
        onClose={() => setTodoId(null)}
        onNotice={onNotice}
      />
    </div>
  );
}
