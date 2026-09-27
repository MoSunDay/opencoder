import { Button, Drawer, Form, Input, Popconfirm, Select, Space, Table, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPost } from '../api.js';
import { err, ok } from '../notice.js';
import { RelationSelect } from './views/relationSelect.jsx';
import { flattenTodos, groupOptions, matchesText, searchSelect } from './model/relations.js';

const { TextArea } = Input;
const STATUS_LABELS = { draft: '待处理', planned: '已规划', done: '已完成' };

function CreateTodo({ open, overview, groupId, onClose, onCreated, onNotice }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const create = async (values) => {
    setSaving(true);
    try {
      const todo = await apiPost('/api/project/todos', {
        title: values.title.trim(),
        draft: values.draft || '',
        milestone_id: values.milestone_id || null,
      });
      form.resetFields();
      onNotice(ok('TODO 已创建'));
      onCreated(todo.id);
    } catch (error) { onNotice(err(error.message)); }
    finally { setSaving(false); }
  };
  return <Drawer open={open} title="新建 TODO" onClose={onClose} size={600} destroyOnHidden
    extra={<Button type="primary" loading={saving} onClick={() => form.submit()}>创建</Button>}>
    <Form form={form} layout="vertical" onFinish={create} initialValues={{ milestone_id: groupId || undefined }}>
      <Form.Item name="title" label="标题" rules={[{ required: true, whitespace: true }]}><Input /></Form.Item>
      <Form.Item name="milestone_id" label="所属里程碑或专项"><Select {...searchSelect} placeholder="未分组" options={groupOptions(overview)} /></Form.Item>
      <Form.Item name="draft" label="任务说明"><TextArea rows={5} /></Form.Item>
    </Form>
  </Drawer>;
}

export function TodosTab({ overview, refresh, openTodo, onNotice, groupFilter, setGroupFilter }) {
  const [createOpen, setCreateOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState(null);
  const rows = flattenTodos(overview).filter((todo) =>
    matchesText(query, todo.title, todo.id)
    && (!status || todo.status === status)
    && (!groupFilter || (groupFilter === 'unlinked' ? !todo.milestone_id : todo.milestone_id === groupFilter)));
  const remove = async (todo) => {
    try {
      await apiDel(`/api/project/todos/${encodeURIComponent(todo.id)}`);
      onNotice(ok('TODO 已删除'));
      await refresh();
    } catch (error) { onNotice(err(error.message)); }
  };
  const columns = [
    { title: 'TODO', dataIndex: 'title', render: (title, todo) => <Button type="link" onClick={() => openTodo(todo.id)}>{title}</Button> },
    { title: '里程碑 / 专项', render: (_, todo) => <RelationSelect path={`/api/project/todos/${encodeURIComponent(todo.id)}`} field="milestone_id"
      value={todo.milestone_id} options={groupOptions(overview)} refresh={refresh} onNotice={onNotice} label={`TODO ${todo.title} 所属分组`} /> },
    { title: '项目', dataIndex: 'goal_title', render: (title) => title || '未关联项目' },
    { title: '状态', dataIndex: 'status', render: (value) => STATUS_LABELS[value] || value },
    { title: '操作', render: (_, todo) => <Space>
      <Button type="link" onClick={() => openTodo(todo.id)}>详情与执行</Button>
      <Popconfirm title="删除该 TODO？" onConfirm={() => remove(todo)}><Button danger type="link">删除</Button></Popconfirm>
    </Space> },
  ];
  return <Space orientation="vertical" style={{ width: '100%' }} size={12}>
    <Space wrap>
      <Button type="primary" onClick={() => setCreateOpen(true)}>新建 TODO</Button>
      <Input.Search aria-label="搜索 TODO" value={query} onChange={(event) => setQuery(event.target.value)} allowClear style={{ width: 230 }} />
      <Select {...searchSelect} aria-label="筛选 TODO 分组" placeholder="全部分组" value={groupFilter} onChange={setGroupFilter}
        style={{ width: 220 }} options={[{ value: 'unlinked', label: '未分组' }, ...groupOptions(overview)]} />
      <Select allowClear aria-label="筛选 TODO 状态" placeholder="全部状态" value={status} onChange={setStatus}
        style={{ width: 150 }} options={Object.entries(STATUS_LABELS).map(([value, label]) => ({ value, label }))} />
    </Space>
    <Typography.Text type="secondary">关联执行仅作为任务证据，不自动改写 TODO 状态。</Typography.Text>
    <Table rowKey="id" dataSource={rows} columns={columns} scroll={{ x: 'max-content' }} />
    <CreateTodo key={createOpen ? groupFilter : 'closed'} open={createOpen} overview={overview}
      groupId={groupFilter === 'unlinked' ? null : groupFilter} onNotice={onNotice}
      onClose={() => setCreateOpen(false)} onCreated={async (id) => { setCreateOpen(false); await refresh(); openTodo(id); }} />
  </Space>;
}
