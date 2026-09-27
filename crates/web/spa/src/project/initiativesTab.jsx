import { Button, Form, Input, Popconfirm, Segmented, Select, Space, Table, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { Markdown } from './markdown.jsx';
import { MdEditDrawer } from './views/mdDrawer.jsx';
import { flattenInitiatives, flattenMilestones, projectOptions, matchesText, searchSelect } from './model/relations.js';
import { RelationSelect } from './views/relationSelect.jsx';
import { err, ok } from '../notice.js';

const STATUS_OPTIONS = [
  { label: '未开始', value: 'planned' },
  { label: '进行中', value: 'in_progress' },
  { label: '已完成', value: 'done' },
];
function GroupTab({ overview, refresh, onNotice, goalFilter, setGoalFilter, openTodos, groupType }) {
  const initiative = groupType === 'initiative';
  const label = initiative ? '专项' : '里程碑';
  const base = initiative ? '/api/project/initiatives' : '/api/project/milestones';
  const path = (id) => `${base}/${encodeURIComponent(id)}`;
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(null);
  const [query, setQuery] = useState('');
  const [status, setStatusFilter] = useState(null);
  const projects = projectOptions(overview);
  const rows = (initiative ? flattenInitiatives(overview) : flattenMilestones(overview)).filter((m) =>
    matchesText(query, m.title, m.id) && (!status || m.status === status)
    && (!goalFilter || (goalFilter === 'unlinked' ? !m.goal_id : m.goal_id === goalFilter)));
  const save = async (values) => {
    const body = { ...values, goal_id: values.goal_id ?? null };
    try {
      if (editing) await apiPatch(path(editing.id), body);
      else await apiPost(base, body);
      setOpen(false); onNotice(ok(`${label}已保存`)); await refresh();
      return true;
    } catch (e) { onNotice(err(`保存${label}失败: ` + e.message)); return false; }
  };
  const setStatus = async (row, next) => {
    try { await apiPatch(path(row.id), { status: next }); await refresh(); }
    catch (e) { onNotice(err(`切换${label}状态失败: ` + e.message)); }
  };
  const remove = async (row) => {
    try { await apiDel(path(row.id)); onNotice(ok(`${label}已删除`)); await refresh(); }
    catch (e) { onNotice(err(`删除${label}失败: ` + e.message)); }
  };
  const columns = [
    { title: label, dataIndex: 'title', render: (title) => <Typography.Text strong>{title}</Typography.Text> },
    { title: '所属项目', key: 'project', render: (_, row) => <RelationSelect key={row.id} path={path(row.id)}
      field="goal_id" value={row.goal_id} options={projects} refresh={refresh} onNotice={onNotice} label={`${label} ${row.title} 所属项目`} /> },
    { title: '状态', dataIndex: 'status', render: (value, row) => <Segmented size="small" value={value} options={STATUS_OPTIONS} onChange={(next) => setStatus(row, next)} /> },
    { title: 'TODO', key: 'todos', render: (_, row) => <Button type="link" onClick={() => openTodos?.(row.id)}>{row.todos?.length || 0} 条 TODO</Button> },
    { title: '操作', key: 'actions', render: (_, row) => <Space>
      <Button type="link" onClick={() => { setEditing(row); setOpen(true); }}>编辑</Button>
      <Popconfirm title={`删除该${label}？`} description={`仅可删除没有关联 TODO 的${label}。`} onConfirm={() => remove(row)} okText="删除" cancelText="取消">
        <Button danger type="link" disabled={!!row.todos?.length}>删除</Button>
      </Popconfirm>
    </Space> },
  ];
  return <Space orientation="vertical" style={{ width: '100%' }} size={16}>
    <Space wrap>
      <Button type="primary" onClick={() => { setEditing(null); setOpen(true); }}>新建{label}</Button>
      <Input.Search aria-label={`搜索${label}`} placeholder="搜索名称或 ID" value={query} onChange={(e) => setQuery(e.target.value)} allowClear style={{ width: 220 }} />
      <Select {...searchSelect} aria-label={`筛选${label}项目`} placeholder="全部项目" style={{ width: 210 }} value={goalFilter}
        options={[{ value: 'unlinked', label: '未关联项目' }, ...projects]} onChange={setGoalFilter} />
      <Select allowClear aria-label={`筛选${label}状态`} placeholder="全部状态" style={{ width: 140 }} value={status} options={STATUS_OPTIONS} onChange={setStatusFilter} />
    </Space>
    <Typography.Text type="secondary">{label}聚拢 TODO，可独立存在，也可关联项目。</Typography.Text>
    <Table rowKey="id" columns={columns} dataSource={rows} scroll={{ x: 'max-content' }}
      locale={{ emptyText: `还没有${label}` }}
      expandable={{ expandedRowRender: (row) => <Markdown text={row.detail_md} /> }} />
    <MdEditDrawer open={open} title={editing ? `编辑${label}` : `新建${label}`}
      initial={editing || { goal_id: goalFilter && goalFilter !== 'unlinked' ? goalFilter : null }}
      extraTop={<Form.Item name="goal_id" label="所属项目"><Select {...searchSelect} aria-label="goal_id" placeholder="可不关联项目" options={projects} /></Form.Item>}
      onCancel={() => setOpen(false)} onOk={save} />
  </Space>;
}

export function InitiativesTab(props) { return <GroupTab {...props} groupType="initiative" />; }
export function MilestonesTab(props) { return <GroupTab {...props} groupType="milestone" />; }
