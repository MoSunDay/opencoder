import { CurrentSummary } from '../summary/view.jsx';
import { ContentSection } from '../summary/section.jsx';
import { PeopleSummary } from '../people/fields.jsx';
import { EditButton } from '../../ui/permissions.jsx';
import { Drawer, Space, Tabs, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiPatch, apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';
import { Markdown } from '../markdown.jsx';
import { ProjectInitiatives } from '../sorting/initiatives.jsx';
import { progressOf } from '../model/catalog.js';
import { TodoProgress } from './progress.jsx';
import { MdEditDrawer } from './mdDrawer.jsx';
import { UnavailableDetail } from '../navigation/unavailable.jsx';
export function ProjectDrawer({ projectId, overview, overviewError, refresh, onNotice, onClose, openInitiative }) {
  const project = overview?.goals?.find((g) => g.id === projectId);
  const [editing, setEditing] = useState(null);
  const [tab, setTab] = useState('overview');
  useEffect(() => { setTab('overview'); setEditing(null); }, [projectId]);
  const rows = (project?.initiatives || []).map((i) => ({ ...i, goal_title: project?.title, progress: progressOf(i.todos) }));
  const save = async (values) => {
    try {
      if (editing === 'project') await apiPatch(`/api/project/goals/${encodeURIComponent(projectId)}`, values);
      else await apiPost('/api/project/initiatives', { ...values, goal_id: projectId });
      setEditing(null); await refresh(); onNotice(ok('已保存')); return true;
    } catch (error) { onNotice(err(error.message)); return false; }
  };
  return <Drawer open={!!projectId} title={`项目 · ${project?.title || projectId || ''}`} onClose={onClose} size="70vw" className="project-detail-drawer" rootClassName="project-project-drawer" destroyOnHidden extra={project && <EditButton onClick={() => setEditing('project')}>编辑项目</EditButton>}>
    {!project && projectId ? <UnavailableDetail label="项目" overview={overview} error={overviewError} refresh={refresh} /> : <Tabs className="project-detail-tabs" activeKey={tab} onChange={setTab} items={[
      { key: 'overview', label: '概览', children: <div className="project-detail-overview">
        <div className="project-detail-meta"><PeopleSummary record={project} />
          <TodoProgress progress={progressOf(rows.flatMap((i) => i.todos || []))} /></div>
        <CurrentSummary record={project} collection="goals" refresh={refresh} />
        <ContentSection key={projectId} title="项目目标"><Markdown text={project?.detail_md} /></ContentSection>
      </div> },
      { key: 'initiatives', label: '子项', children: <div className="project-detail-overview">
        <Space wrap><Typography.Title level={5} style={{ margin: 0 }}>子项进度</Typography.Title><EditButton onClick={() => setEditing('initiative')}>新建子项</EditButton></Space>
        <ProjectInitiatives key={projectId} projectId={projectId} rows={rows} refresh={refresh} onNotice={onNotice} openInitiative={openInitiative} />
      </div> },
    ]} />}
    <MdEditDrawer open={!!editing} title={editing === 'project' ? '编辑项目' : '新建子项'} detailLabel={editing === 'project' ? '项目目标' : '子项目标'} initial={editing === 'project' ? project : null} onCancel={() => setEditing(null)} onOk={save} />
  </Drawer>;
}
