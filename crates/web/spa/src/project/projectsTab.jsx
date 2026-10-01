import { Button, Dropdown, Modal, Space, Tag } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { MdEditDrawer } from './views/mdDrawer.jsx';
import { ProjectTable, TableText, dateColumn } from './views/projectTable.jsx';
import { TodoProgress } from './views/progress.jsx';
import { progressOf } from './model/catalog.js';
import { err, ok } from '../notice.js';

export function ProjectsTab({ overview, refresh, onNotice, openProject }) {
  const [modal, contextHolder] = Modal.useModal();
  const [editing, setEditing] = useState(null);
  const [open, setOpen] = useState(false);
  const rows = (overview?.goals || []).map((g) => ({ ...g, progress: progressOf((g.initiatives || []).flatMap((i) => i.todos || [])) }));
  const save = async (body) => {
    try {
      if (editing) await apiPatch(`/api/project/goals/${encodeURIComponent(editing.id)}`, body);
      else await apiPost('/api/project/goals', body);
      setOpen(false); await refresh(); onNotice(ok('项目已保存')); return true;
    } catch (error) { onNotice(err(error.message)); return false; }
  };
  const remove = async (row) => {
    try { await apiDel(`/api/project/goals/${encodeURIComponent(row.id)}`); await refresh(); onNotice(ok('项目已删除，专项已转为独立专项')); }
    catch (error) { onNotice(err(error.message)); }
  };
  const toggle = async (row) => {
    try { await apiPatch(`/api/project/goals/${encodeURIComponent(row.id)}`, { status: row.status === 'archived' ? 'active' : 'archived' }); await refresh(); }
    catch (error) { onNotice(err(error.message)); }
  };
  const columns = [
    { title: '项目', key: 'title', searchValue: (r) => r.title, render: (_, r) => <Button type="link" className="project-name" onClick={(e) => { e.stopPropagation(); openProject(r.id); }}><TableText>{r.title}</TableText></Button> },
    { title: '状态', key: 'status', width: '12%', kind: 'enum', searchValue: (r) => r.status === 'archived' ? '已归档' : '进行中', render: (_, r) => <Tag>{r.status === 'archived' ? '已归档' : '进行中'}</Tag> },
    { title: '专项数', key: 'groups', width: '11%', kind: 'number', searchValue: (r) => r.initiatives?.length || 0, render: (_, r) => r.initiatives?.length || 0 },
    { title: 'TODO 数', key: 'todos', width: '11%', kind: 'number', searchValue: (r) => r.progress.total, render: (_, r) => r.progress.total },
    { title: '总进度', key: 'progress', width: '18%', kind: 'number', searchValue: (r) => r.progress.percent, render: (_, r) => <TodoProgress progress={r.progress} /> },
    dateColumn,
    { title: '操作', key: 'actions', width: '12%', render: (_, row) => <div onClick={(e) => e.stopPropagation()}><Dropdown menu={{ items: [
      { key: 'edit', label: '编辑', onClick: () => { setEditing(row); setOpen(true); } },
      { key: 'archive', label: row.status === 'archived' ? '激活' : '归档', onClick: () => toggle(row) },
      { key: 'delete', label: '删除', danger: true, onClick: () => modal.confirm({ title: '删除该项目？', content: '专项变为独立专项，TODO 和执行记录保留；项目 Tag 会重新匹配。', onOk: () => remove(row) }) },
    ] }} trigger={['click']}><Button size="small">操作</Button></Dropdown></div> },
  ];
  return <Space orientation="vertical" style={{ width: '100%' }} size={12}>
    {contextHolder}
    <Button type="primary" onClick={() => { setEditing(null); setOpen(true); }}>新建项目</Button>
    <ProjectTable label="项目表格" rows={rows} columns={columns} onRowClick={(row) => openProject(row.id)} locale={{ emptyText: '还没有项目' }} />
    <MdEditDrawer open={open} title={editing ? '编辑项目' : '新建项目'} initial={editing} onCancel={() => setOpen(false)} onOk={save} />
  </Space>;
}
