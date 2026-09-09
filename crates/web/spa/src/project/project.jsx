// project.jsx — 菜单页「项目」: 项目模块总控。The /api/project/overview
// state + adaptive polling (3s while any todo is running, else 8s) live in
// the shared useOverview hook (iteration 4) so 进展 / Owner 视角 ride the
// same snapshot; this panel renders four tabs: 总览 / 项目目标 / 里程碑 /
// TODO. `refresh` is a silent reload handed to every tab so writes converge
// into the single overview snapshot. The TODO drawer (详情/生成Plan 跳转) is
// owned here and keyed by todoId.

import { Alert, Card, Col, Row, Spin, Statistic, Tabs, Typography } from 'antd';
import { useState } from 'react';
import { PageShell } from '../shell/pageShell.jsx';
import { GoalsTab } from './goalsTab.jsx';
import { MilestonesTab } from './milestonesTab.jsx';
import { TodoDrawer } from './todoDrawer.jsx';
import { TodosTab } from './todosTab.jsx';
import { flattenMilestones, flattenTodos } from './model/relations.js';
import { useOverview } from './useOverview.js';

const { Text, Paragraph } = Typography;

/// 总览 tab: counters + the workflow hint. Pure function of `overview`.
function OverviewTab({ overview }) {
  const goals = (overview && overview.goals) || [];
  const backlog = (overview && overview.backlog) || [];
  const milestones = flattenMilestones(overview);
  const todos = flattenTodos(overview);
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
              plan，输出与 session 归档）→ 每次运行都有版本可回看。里程碑聚拢专项 TODO，可独立存在或关联项目；TODO 也可独立记录。
            </Text>
          </Paragraph>
        </Card>
      </Col>
    </Row>
  );
}

export function ProjectPanel({ onNotice }) {
  const { overview, loading, refresh, error, updated } = useOverview({ onNotice });
  const [activeTab, setActiveTab] = useState('overview');
  const [goalFilter, setGoalFilter] = useState(null);
  const [milestoneFilter, setMilestoneFilter] = useState(null);
  const [todoId, setTodoId] = useState(null); // open TODO drawer

  const tabs = [
    { key: 'overview', label: '总览', children: <OverviewTab overview={overview} /> },
    { key: 'goals', label: '项目目标', children: <GoalsTab openMilestones={(id) => { setGoalFilter(id); setActiveTab('milestones'); }} overview={overview} refresh={refresh} onNotice={onNotice} /> },
    { key: 'milestones', label: '里程碑', children: <MilestonesTab goalFilter={goalFilter} setGoalFilter={setGoalFilter} openTodos={(id) => { setMilestoneFilter(id); setActiveTab('todos'); }} overview={overview} refresh={refresh} onNotice={onNotice} /> },
    { key: 'todos', label: 'TODO', children: <TodosTab milestoneFilter={milestoneFilter} setMilestoneFilter={setMilestoneFilter} overview={overview} refresh={refresh} openTodo={setTodoId} onNotice={onNotice} /> },
  ];

  return (
    <div>
      {error && <Alert type="error" showIcon title={error} description={updated ? `最近成功读取：${new Date(updated).toLocaleString()}` : null} />}
      <Spin spinning={loading}>
        <Tabs activeKey={activeTab} onChange={setActiveTab} items={tabs} />
      </Spin>
      <TodoDrawer
        todoId={todoId}
        overview={overview}
        refresh={refresh}
        onClose={() => setTodoId(null)}
        onNotice={onNotice}
      />
    </div>
  );
}
