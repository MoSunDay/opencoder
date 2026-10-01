import { Alert, Spin, Tabs } from 'antd';
import { useState } from 'react';
import { PageShell } from '../shell/pageShell.jsx';
import { ProjectsTab } from './projectsTab.jsx';
import { InitiativesTab } from './initiativesTab.jsx';
import { TodosTab } from './todosTab.jsx';
import { TodoDrawer } from './todoDrawer.jsx';
import { ProjectDrawer } from './views/projectDrawer.jsx';
import { InitiativeDrawer } from './views/initiativeDrawer.jsx';
import { useOverview } from './useOverview.js';
import './project.css';
import { ProjectViewState } from './views/viewState.jsx';
export function ProjectPanel(props) { return <ProjectViewState><ProjectPage {...props} /></ProjectViewState>; }

function ProjectPage({ onNotice }) {
  const { overview, loading, refresh, error } = useOverview({ onNotice });
  const [tab, setTab] = useState('projects');
  const [projectId, setProjectId] = useState(null);
  const [initiativeId, setInitiativeId] = useState(null);
  const [todoId, setTodoId] = useState(null);
  const shared = { overview, refresh, onNotice };
  const tabs = [
    { key: 'projects', label: '项目', children: <ProjectsTab {...shared} openProject={setProjectId} /> },
    { key: 'initiatives', label: '专项', children: <InitiativesTab {...shared} openInitiative={setInitiativeId} /> },
    { key: 'todos', label: 'TODO', children: <TodosTab {...shared} openTodo={setTodoId} /> },
  ];
  return <PageShell page="project">
    {error && <Alert type="error" showIcon title={error} />}
    <Spin spinning={loading}><Tabs activeKey={tab} onChange={setTab} items={tabs} /></Spin>
    <ProjectDrawer {...shared} projectId={projectId} onClose={() => setProjectId(null)} openInitiative={setInitiativeId} />
    <InitiativeDrawer {...shared} initiativeId={initiativeId} onClose={() => setInitiativeId(null)} openTodo={setTodoId} />
    <TodoDrawer {...shared} todoId={todoId} onClose={() => setTodoId(null)} />
  </PageShell>;
}
