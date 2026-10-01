import { Button, Popconfirm, Select, Space, Tag } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch } from '../api.js';
import { err, ok } from '../notice.js';
import { LANES, laneOf } from './model/board.js';
import { flattenTodos } from './model/relations.js';
import { todoTags } from './model/catalog.js';
import { ProjectTable, TableText, dateColumn } from './views/projectTable.jsx';
import { CreateTodo } from './views/todoForm.jsx';

export function TodosTab({ overview, refresh, openTodo, onNotice }) {
  const [createOpen, setCreateOpen] = useState(false);
  const [busyId, setBusyId] = useState(null);
  const rows = flattenTodos(overview);
  const remove = async (todo) => {
    try { await apiDel(`/api/project/todos/${encodeURIComponent(todo.id)}`); await refresh(); onNotice(ok('TODO 已删除')); }
    catch (error) { onNotice(err(error.message)); }
  };
  const setStatus = async (todo, status) => {
    setBusyId(todo.id);
    try { await apiPatch(`/api/project/todos/${encodeURIComponent(todo.id)}`, { board_status: status }); await refresh(); }
    catch (error) { onNotice(err(error.message)); } finally { setBusyId(null); }
  };
  const columns = [
    { title: 'TODO', key: 'title', searchValue: (r) => r.title, render: (_, r) => <Button type="link" className="project-name" onClick={(e) => { e.stopPropagation(); openTodo(r.id); }}><TableText>{r.title}</TableText></Button> },
    { title: '所属专项', key: 'initiative', width: '15%', kind: 'enum', searchValue: (r) => r.group_title || '未关联', render: (_, r) => <TableText>{r.group_title}</TableText> },
    { title: '所属项目', key: 'project', width: '15%', kind: 'enum', searchValue: (r) => r.goal_title || '未关联', render: (_, r) => <TableText>{r.goal_title}</TableText> },
    { title: '状态', key: 'status', width: '15%', kind: 'enum', searchValue: (r) => LANES.find(([id]) => id === laneOf(r))?.[1], render: (_, r) => <div onClick={(e) => e.stopPropagation()}><Select size="small" aria-label={`${r.title}状态`} value={laneOf(r)} disabled={busyId === r.id} loading={busyId === r.id} onChange={(v) => setStatus(r, v)} style={{ width: '100%' }} options={LANES.map(([value, label]) => ({ value, label }))} /></div> },
    { title: 'Tag', key: 'tags', width: '16%', kind: 'enum', searchValue: (r) => todoTags(overview, r).map((tag) => tag.name), render: (_, r) => <Space wrap size={[0, 4]}>{todoTags(overview, r).map((tag) => <Tag key={tag.id}>{tag.name}</Tag>)}</Space> },
    dateColumn,
    { title: '操作', key: 'actions', width: '11%', render: (_, r) => <div onClick={(e) => e.stopPropagation()}><Popconfirm title="删除该 TODO？" onConfirm={() => remove(r)}><Button type="link" danger size="small">删除</Button></Popconfirm></div> },
  ];
  return <Space orientation="vertical" style={{ width: '100%' }} size={12}>
    <Button type="primary" onClick={() => setCreateOpen(true)}>新建 TODO</Button>
    <ProjectTable label="TODO 表格" columns={columns} rows={rows} onRowClick={(r) => openTodo(r.id)} locale={{ emptyText: '还没有 TODO' }} />
    <CreateTodo key={createOpen ? 'open' : 'closed'} open={createOpen} overview={overview} onNotice={onNotice} onClose={() => setCreateOpen(false)} onCreated={async (id) => { setCreateOpen(false); await refresh(); openTodo(id); }} />
  </Space>;
}
