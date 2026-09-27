import { Alert, Card, Col, Row, Spin, Statistic, Tabs } from 'antd';
import { useState } from 'react';
import { PageShell } from '../shell/pageShell.jsx';
import { ProjectsTab } from './projectsTab.jsx';
import { InitiativesTab, MilestonesTab } from './initiativesTab.jsx';
import { TodosTab } from './todosTab.jsx';
import { TodoDrawer } from './todoDrawer.jsx';
import { flattenInitiatives, flattenMilestones, flattenTodos } from './model/relations.js';
import { useOverview } from './useOverview.js';

function Summary({ overview }) {
  const todos = flattenTodos(overview);
  const cards = [
    ['项目', overview?.goals?.length || 0],
    ['里程碑', flattenMilestones(overview).length],
    ['专项', flattenInitiatives(overview).length],
    ['TODO', todos.length],
  ];
  return <Row gutter={[12, 12]} style={{ marginBottom: 16 }}>
    {cards.map(([title, value]) => <Col key={title} xs={12} md={6}>
      <Card size="small"><Statistic title={title} value={value} /></Card>
    </Col>)}
  </Row>;
}

export function ProjectPanel({ onNotice }) {
  const { overview, loading, refresh, error } = useOverview({ onNotice });
  const [tab, setTab] = useState('projects');
  const [projectFilter, setProjectFilter] = useState(null);
  const [groupFilter, setGroupFilter] = useState(null);
  const [todoId, setTodoId] = useState(null);
  const tabs = [
    { key: 'projects', label: '项目', children: <ProjectsTab overview={overview} refresh={refresh} onNotice={onNotice} openMilestones={(id) => { setProjectFilter(id); setTab('milestones'); }} openInitiatives={(id) => { setProjectFilter(id); setTab('initiatives'); }} /> },
    { key: 'milestones', label: '里程碑', children: <MilestonesTab overview={overview} refresh={refresh} onNotice={onNotice} goalFilter={projectFilter} setGoalFilter={setProjectFilter} openTodos={(id) => { setGroupFilter(id); setTab('todos'); }} /> },
    { key: 'initiatives', label: '专项', children: <InitiativesTab overview={overview} refresh={refresh} onNotice={onNotice} goalFilter={projectFilter} setGoalFilter={setProjectFilter} openTodos={(id) => { setGroupFilter(id); setTab('todos'); }} /> },
    { key: 'todos', label: 'TODO', children: <TodosTab overview={overview} refresh={refresh} onNotice={onNotice} groupFilter={groupFilter} setGroupFilter={setGroupFilter} openTodo={setTodoId} /> },
  ];
  return <PageShell page="project">
    {error && <Alert type="error" showIcon title={error} />}
    <Spin spinning={loading}>
      <Summary overview={overview} />
      <Tabs activeKey={tab} onChange={setTab} items={tabs} />
    </Spin>
    <TodoDrawer todoId={todoId} overview={overview} refresh={refresh} onClose={() => setTodoId(null)} onNotice={onNotice} />
  </PageShell>;
}
