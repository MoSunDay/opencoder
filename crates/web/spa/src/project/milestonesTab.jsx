import { Button, Form, Input, Popconfirm, Segmented, Select, Space, Table, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { Markdown } from './markdown.jsx';
import { MdEditModal } from './views/mdModal.jsx';
import { flattenMilestones, goalOptions, matchesText, searchSelect } from './model/relations.js';
import { RelationSelect } from './views/relationSelect.jsx';
import { err, ok } from '../notice.js';

const STATUS_OPTIONS = [
  { label: '未开始', value: 'planned' },
  { label: '进行中', value: 'in_progress' },
  { label: '已完成', value: 'done' },
];
const msPath = (id) => '/api/project/milestones/' + encodeURIComponent(id);

export function MilestonesTab({ overview, refresh, onNotice, goalFilter, setGoalFilter, openTodos }) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(null);
  const [query, setQuery] = useState('');
  const [status, setStatusFilter] = useState(null);
  const projects = goalOptions(overview);
  const rows = flattenMilestones(overview).filter((m) =>
    matchesText(query, m.title, m.id) && (!status || m.status === status)
    && (!goalFilter || (goalFilter === 'unlinked' ? !m.goal_id : m.goal_id === goalFilter)));
  const save = async (values) => {
    const body = { ...values, goal_id: values.goal_id ?? null };
    try {
      if (editing) await apiPatch(msPath(editing.id), body);
      else await apiPost('/api/project/milestones', body);
      setOpen(false); onNotice(ok('里程碑已保存')); await refresh();
      return true;
    } catch (e) { onNotice(err('保存里程碑失败: ' + e.message)); return false; }
  };
  const setStatus = async (row, next) => {
    try { await apiPatch(msPath(row.id), { status: next }); await refresh(); }
    catch (e) { onNotice(err('切换里程碑状态失败: ' + e.message)); }
  };
  const remove = async (row) => {
    try { await apiDel(msPath(row.id)); onNotice(ok('里程碑已删除')); await refresh(); }
    catch (e) { onNotice(err('删除里程碑失败: ' + e.message)); }
  };
  const columns = [
    { title: '里程碑', dataIndex: 'title', render: (title) => <Typography.Text strong>{title}</Typography.Text> },
    { title: '所属项目', key: 'project', render: (_, row) => <RelationSelect key={row.id} path={msPath(row.id)}
      field="goal_id" value={row.goal_id} options={projects} refresh={refresh} onNotice={onNotice} label={`里程碑 ${row.title} 所属项目`} /> },
    { title: '状态', dataIndex: 'status', render: (value, row) => <Segmented size="small" value={value} options={STATUS_OPTIONS} onChange={(next) => setStatus(row, next)} /> },
    { title: 'TODO', key: 'todos', render: (_, row) => <Button type="link" onClick={() => openTodos?.(row.id)}>{row.todos?.length || 0} 条 TODO</Button> },
    { title: '操作', key: 'actions', render: (_, row) => <Space>
      <Button type="link" onClick={() => { setEditing(row); setOpen(true); }}>编辑</Button>
      <Popconfirm title="删除该里程碑？" description="仅可删除没有关联 TODO 的里程碑。" onConfirm={() => remove(row)} okText="删除" cancelText="取消">
        <Button danger type="link" disabled={!!row.todos?.length}>删除</Button>
      </Popconfirm>
    </Space> },
  ];
  return <Space orientation="vertical" style={{ width: '100%' }} size={16}>
    <Space wrap>
      <Button type="primary" onClick={() => { setEditing(null); setOpen(true); }}>新建里程碑</Button>
      <Input.Search aria-label="搜索里程碑" placeholder="搜索名称或 ID" value={query} onChange={(e) => setQuery(e.target.value)} allowClear style={{ width: 220 }} />
      <Select {...searchSelect} aria-label="筛选里程碑项目" placeholder="全部项目" style={{ width: 210 }} value={goalFilter}
        options={[{ value: 'unlinked', label: '独立专项（无项目）' }, ...projects]} onChange={setGoalFilter} />
      <Select allowClear aria-label="筛选里程碑状态" placeholder="全部状态" style={{ width: 140 }} value={status} options={STATUS_OPTIONS} onChange={setStatusFilter} />
    </Space>
    <Typography.Text type="secondary">里程碑用于聚拢专项 TODO，可独立存在，也可关联项目。</Typography.Text>
    <Table rowKey="id" columns={columns} dataSource={rows} scroll={{ x: 'max-content' }}
      locale={{ emptyText: '还没有里程碑，可直接创建独立专项' }}
      expandable={{ expandedRowRender: (row) => <Markdown text={row.detail_md} /> }} />
    <MdEditModal open={open} title={editing ? '编辑里程碑' : '新建里程碑'}
      initial={editing || { goal_id: goalFilter && goalFilter !== 'unlinked' ? goalFilter : null }}
      extraTop={<Form.Item name="goal_id" label="所属项目"><Select {...searchSelect} aria-label="goal_id" placeholder="可不关联项目" options={projects} /></Form.Item>}
      onCancel={() => setOpen(false)} onOk={save} />
  </Space>;
}
