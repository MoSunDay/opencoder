import { EditButton } from '../ui/permissions.jsx';
import { Button, Form, Popconfirm, Select, Space, Tag } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { MdEditDrawer } from './views/mdDrawer.jsx';
import { ProjectTable, TableText, dateColumn } from './views/projectTable.jsx';
import { TodoProgress } from './views/progress.jsx';
import { flattenInitiatives, projectOptions, searchSelect } from './model/relations.js';
import { progressOf } from './model/catalog.js';
import { err, ok } from '../notice.js';

const statusLabel = (status) => ({ planned: '未开始', in_progress: '进行中', done: '已完成' }[status] || status);
export function initiativeColumns({ openInitiative }) {
  return [
    { title: '专项', key: 'title', searchValue: (r) => r.title, render: (_, r) => <Button type="link" className="project-name" onClick={(e) => { e.stopPropagation(); openInitiative(r.id); }}><TableText>{r.title}</TableText></Button> },
    { title: '所属项目', key: 'project', width: '20%', kind: 'enum', searchValue: (r) => r.goal_title || '未关联', render: (_, r) => <TableText>{r.goal_title}</TableText> },
    { title: '状态', key: 'status', width: '13%', kind: 'enum', searchValue: (r) => statusLabel(r.status), render: (_, r) => <Tag>{statusLabel(r.status)}</Tag> },
    { title: 'TODO 数', key: 'todos', width: '11%', kind: 'number', searchValue: (r) => r.progress.total, render: (_, r) => r.progress.total },
    { title: '进度', key: 'progress', width: '20%', kind: 'number', searchValue: (r) => r.progress.percent, render: (_, r) => <TodoProgress progress={r.progress} /> },
  ];
}
export function InitiativesTab({ overview, refresh, onNotice, openInitiative }) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(null);
  const rows = flattenInitiatives(overview).map((i) => ({ ...i, progress: progressOf(i.todos) }));
  const save = async (values) => {
    try {
      const body = { ...values, goal_id: values.goal_id ?? null };
      if (editing) await apiPatch(`/api/project/initiatives/${encodeURIComponent(editing.id)}`, body);
      else await apiPost('/api/project/initiatives', body);
      setOpen(false); await refresh(); onNotice(ok('专项已保存，TODO 标签已按归属重新匹配')); return true;
    } catch (error) { onNotice(err(error.message)); return false; }
  };
  const remove = async (row) => {
    try { await apiDel(`/api/project/initiatives/${encodeURIComponent(row.id)}`); await refresh(); onNotice(ok('专项已删除')); }
    catch (error) { onNotice(err(error.message)); }
  };
  const columns = [...initiativeColumns({ openInitiative }), dateColumn, { title: '操作', key: 'actions', width: '15%', render: (_, row) => <Space onClick={(e) => e.stopPropagation()} wrap size={0}>
    <EditButton type="link" size="small" onClick={() => { setEditing(row); setOpen(true); }}>编辑</EditButton>
    <Popconfirm title="删除该专项？" onConfirm={() => remove(row)}><EditButton danger type="link" size="small" disabled={!!row.todos?.length} title={row.todos?.length ? '先迁移或移除 TODO，再删除专项' : ''}>删除</EditButton></Popconfirm>
  </Space> }];
  return <Space orientation="vertical" style={{ width: '100%' }} size={12}>
    <EditButton type="primary" onClick={() => { setEditing(null); setOpen(true); }}>新建专项</EditButton>
    <ProjectTable label="专项表格" rows={rows} columns={columns} onRowClick={(row) => openInitiative(row.id)} locale={{ emptyText: '还没有专项' }} />
    <MdEditDrawer open={open} title={editing ? '编辑专项' : '新建专项'} initial={editing}
      extraTop={<><Form.Item name="goal_id" label="所属项目"><Select {...searchSelect} aria-label="goal_id" placeholder="独立专项" options={projectOptions(overview)} /></Form.Item><Form.Item name="status" label="状态"><Select options={['planned', 'in_progress', 'done'].map((value) => ({ value, label: statusLabel(value) }))} /></Form.Item></>}
      onCancel={() => setOpen(false)} onOk={save} />
  </Space>;
}
