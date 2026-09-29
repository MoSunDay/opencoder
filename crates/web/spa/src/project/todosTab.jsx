import { Button, Card, Drawer, Form, Input, Popconfirm, Select, Space, Tag, Typography } from 'antd';
import { MenuOutlined } from '@ant-design/icons';
import { DndContext, KeyboardSensor, PointerSensor, closestCorners, useDroppable, useSensor, useSensors } from '@dnd-kit/core';
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useState } from 'react';
import { apiDel, apiPost, apiPut } from '../api.js';
import { err, ok } from '../notice.js';
import { assignmentBadge, LANES, laneOf, moveTodo, ordered } from './model/board.js';
import { flattenTodos, groupOptions, matchesText, searchSelect } from './model/relations.js';
import { CAPABILITIES } from './execute/launcher.jsx';

const todoPath = (id) => `/api/project/todos/${encodeURIComponent(id)}`;

function CreateTodo({ open, overview, groupId, onClose, onCreated, onNotice }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const create = async (values) => {
    setSaving(true);
    try {
      const todo = await apiPost('/api/project/todos', { title: values.title.trim(), draft: values.draft || '', milestone_id: values.milestone_id || null, capability_id: values.capability_id || null });
      form.resetFields(); onNotice(ok('TODO 已创建')); onCreated(todo.id);
    } catch (error) { onNotice(err(error.message)); }
    finally { setSaving(false); }
  };
  return <Drawer open={open} title="新建 TODO" onClose={onClose} size={600} destroyOnHidden
    extra={<Button type="primary" loading={saving} onClick={() => form.submit()}>创建</Button>}>
    <Form form={form} layout="vertical" onFinish={create} initialValues={{ milestone_id: groupId || undefined }}>
      <Form.Item name="title" label="标题" rules={[{ required: true, whitespace: true }]}><Input /></Form.Item>
      <Form.Item name="milestone_id" label="所属里程碑或专项"><Select {...searchSelect} placeholder="未分组" options={groupOptions(overview)} /></Form.Item>
      <Form.Item name="capability_id" label="执行能力"><Select allowClear placeholder="稍后选择" options={CAPABILITIES} /></Form.Item>
      <Form.Item name="draft" label="任务说明"><Input.TextArea rows={5} /></Form.Item>
    </Form>
  </Drawer>;
}

function TodoCard({ todo, onOpen, onDelete, disabled }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: todo.id, disabled });
  const badge = assignmentBadge(todo.latest_assignment);
  return <div ref={setNodeRef} style={{ transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.5 : 1, marginBottom: 10 }}>
    <Card size="small" title={<Button type="link" onClick={() => onOpen(todo.id)}>{todo.title}</Button>}
      extra={<Button type="text" aria-label={`拖动 ${todo.title}`} icon={<MenuOutlined />} {...attributes} {...listeners} />}>
      <Space orientation="vertical" size={6} style={{ width: '100%' }}>
        {todo.draft && <Typography.Paragraph ellipsis={{ rows: 2 }} style={{ margin: 0 }}>{todo.draft}</Typography.Paragraph>}
        <Space wrap>{todo.group_title && <Tag>{todo.group_title}</Tag>}{todo.capability_id && <Tag>{CAPABILITIES.find((item) => item.value === todo.capability_id)?.label || todo.capability_id}</Tag>}{badge && <Tag color={badge.color}>{badge.label}</Tag>}</Space>
        <Space><Button size="small" onClick={() => onOpen(todo.id)}>详情与执行</Button>
          <Popconfirm title="删除该 TODO？" onConfirm={() => onDelete(todo)}><Button size="small" danger>删除</Button></Popconfirm></Space>
      </Space>
    </Card>
  </div>;
}

function Lane({ status, title, rows, onOpen, onDelete, busy }) {
  const { setNodeRef, isOver } = useDroppable({ id: `lane:${status}` });
  return <section ref={setNodeRef} aria-label={title} style={{ width: 285, flex: '0 0 285px', minHeight: 220, padding: 12, borderRadius: 8, background: isOver ? '#e6f4ff' : '#f5f5f5' }}>
    <Typography.Title level={5}>{title} <Tag>{rows.length}</Tag></Typography.Title>
    <SortableContext items={rows.map((todo) => todo.id)} strategy={verticalListSortingStrategy}>
      {rows.map((todo) => <TodoCard key={todo.id} todo={todo} onOpen={onOpen} onDelete={onDelete} disabled={busy} />)}
    </SortableContext>
  </section>;
}

export function TodosTab({ overview, refresh, openTodo, onNotice, groupFilter, setGroupFilter }) {
  const [createOpen, setCreateOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState(null);
  const all = preview || flattenTodos(overview);
  const shown = all.filter((todo) => matchesText(query, todo.title, todo.id, todo.draft)
    && (!groupFilter || (groupFilter === 'unlinked' ? !todo.milestone_id : todo.milestone_id === groupFilter)));
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));
  const remove = async (todo) => {
    try { await apiDel(todoPath(todo.id)); onNotice(ok('TODO 已删除')); await refresh(); }
    catch (error) { onNotice(err(error.message)); }
  };
  const move = async ({ active, over }) => {
    if (!over || active.id === over.id || busy || query || groupFilter) return;
    const result = moveTodo(all, active.id, over.id);
    if (!result) return;
    setPreview(result.preview);
    setBusy(true);
    try {
      await apiPut('/api/project/todos/order', { board_status: result.status, ids: result.ids });
      await refresh();
    } catch (error) { onNotice(err(error.message)); await refresh(); }
    finally { setPreview(null); setBusy(false); }
  };
  return <Space orientation="vertical" style={{ width: '100%' }} size={12}>
    <Space wrap>
      <Button type="primary" onClick={() => setCreateOpen(true)}>新建 TODO</Button>
      <Input.Search aria-label="搜索 TODO" value={query} onChange={(event) => setQuery(event.target.value)} allowClear style={{ width: 230 }} />
      <Select {...searchSelect} aria-label="筛选 TODO 分组" placeholder="全部分组" value={groupFilter} onChange={setGroupFilter}
        style={{ width: 220 }} options={[{ value: 'unlinked', label: '未分组' }, ...groupOptions(overview)]} />
    </Space>
    {(query || groupFilter) && <Typography.Text type="secondary">清除筛选后可拖动 TODO 排序。</Typography.Text>}
    <DndContext sensors={sensors} collisionDetection={closestCorners} onDragEnd={move}>
      <div style={{ display: 'flex', gap: 12, overflowX: 'auto', paddingBottom: 12 }}>
        {LANES.map(([status, title]) => <Lane key={status} status={status} title={title}
          rows={ordered(shown.filter((todo) => laneOf(todo) === status))} onOpen={openTodo} onDelete={remove} busy={busy || !!query || !!groupFilter} />)}
      </div>
    </DndContext>
    <CreateTodo key={createOpen ? groupFilter : 'closed'} open={createOpen} overview={overview}
      groupId={groupFilter === 'unlinked' ? null : groupFilter} onNotice={onNotice}
      onClose={() => setCreateOpen(false)} onCreated={async (id) => { setCreateOpen(false); await refresh(); openTodo(id); }} />
  </Space>;
}
