import { Button, Drawer, Space, Typography } from 'antd';
import { useState } from 'react';
import { apiPatch, apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';
import { Markdown } from '../markdown.jsx';
import { initiativeColumns } from '../initiativesTab.jsx';
import { progressOf } from '../model/catalog.js';
import { ProjectTable } from './projectTable.jsx';
import { TodoProgress } from './progress.jsx';
import { MdEditDrawer } from './mdDrawer.jsx';
export function ProjectDrawer({ projectId, overview, refresh, onNotice, onClose, openInitiative }) {
  const project = overview?.goals?.find((g) => g.id === projectId);
  const [editing, setEditing] = useState(null);
  const rows = (project?.initiatives || []).map((i) => ({ ...i, goal_title: project?.title, progress: progressOf(i.todos) }));
  const save = async (values) => {
    try {
      if (editing === 'project') await apiPatch(`/api/project/goals/${encodeURIComponent(projectId)}`, values);
      else await apiPost('/api/project/initiatives', { ...values, goal_id: projectId });
      setEditing(null); await refresh(); onNotice(ok('已保存')); return true;
    } catch (error) { onNotice(err(error.message)); return false; }
  };
  return <Drawer open={!!projectId} title={`项目 · ${project?.title || ''}`} onClose={onClose} size="min(880px, 100vw)">
    {!project && projectId ? <Typography.Text type="secondary">项目已删除或正在加载</Typography.Text> : <Space orientation="vertical" size={20} style={{ width: '100%' }}>
      <Button onClick={() => setEditing('project')}>编辑项目</Button>
      <Markdown text={project?.detail_md} /><TodoProgress progress={progressOf(rows.flatMap((i) => i.todos || []))} />
      <Space><Typography.Title level={5} style={{ margin: 0 }}>专项进度</Typography.Title><Button onClick={() => setEditing('initiative')}>新建专项</Button></Space>
      <ProjectTable label="项目内专项表格" columns={initiativeColumns({ openInitiative })} rows={rows} pagination={false} onRowClick={(r) => openInitiative(r.id)} locale={{ emptyText: '这个项目还没有专项' }} />
    </Space>}
    <MdEditDrawer open={!!editing} title={editing === 'project' ? '编辑项目' : '新建专项'} initial={editing === 'project' ? project : null} onCancel={() => setEditing(null)} onOk={save} />
  </Drawer>;
}
