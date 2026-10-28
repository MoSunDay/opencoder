import { Alert, Tabs } from 'antd';
import { useEffect, useState } from 'react';
import { useLinkedLocation } from '../fleet/navigation/location.js';
import { pushLocation } from '../fleet/navigation/history.js';
import { projectLocation, projectTab } from './navigation/links.js';
import { projectHierarchy } from './navigation/hierarchy.js';
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
  const { project: selection } = useLinkedLocation();
  const { projectId, initiativeId, todoId } = projectHierarchy(overview, selection);
  const [tab, setTab] = useState(() => projectTab(selection) || 'projects');
  useEffect(() => { const next = projectTab(selection); if (next) setTab(next); }, [selection]);
  const navigate = (next) => pushLocation(projectLocation(window.location.href, next));
  const setProjectId = (id) => navigate({ projectId: id });
  const setInitiativeId = (id) => navigate({ projectId, initiativeId: id });
  const setTodoId = (id) => navigate({ projectId, initiativeId, todoId: id });
  const shared = { overview, loading, refresh, onNotice, overviewError: error };
  const tabs = [
    { key: 'projects', label: '项目', children: <ProjectsTab {...shared} openProject={setProjectId} /> },
    { key: 'initiatives', label: '子项', children: <InitiativesTab {...shared} openInitiative={setInitiativeId} /> },
    { key: 'todos', label: 'TODO', children: <TodosTab {...shared} openTodo={setTodoId} /> },
  ];
  return <PageShell page="project">
    {error && <Alert type="error" showIcon title={error} />}
    <Tabs activeKey={tab} onChange={setTab} items={tabs} />
    <ProjectDrawer {...shared} active={!initiativeId && !todoId} projectId={projectId} onClose={() => setProjectId(null)} openInitiative={setInitiativeId} />
    <InitiativeDrawer {...shared} active={!todoId} initiativeId={initiativeId} onClose={() => setInitiativeId(null)} openTodo={setTodoId} />
    <TodoDrawer {...shared} todoId={todoId} onClose={() => setTodoId(null)} />
  </PageShell>;
}
