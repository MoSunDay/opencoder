import { EditButton, useCanEdit } from '../../ui/permissions.jsx';
import { Alert, Button, Drawer, Form, Input, Select } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';
import { groupOptions, searchSelect } from '../model/relations.js';
import { effectiveTags } from '../model/catalog.js';
import { LANES } from '../model/board.js';
import { PeopleFields } from '../people/fields.jsx';
import { TodoActions, TodoSection } from '../forms/section.jsx';

export function CreateTodo({ open, overview, initiativeId, onClose, onCreated, onNotice }) {
  const canEdit = useCanEdit();
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const selected = Form.useWatch('initiative_id', form);
  const tags = effectiveTags(overview, selected);
  const create = async (values) => {
    if (!canEdit || saving) return;
    setSaving(true); setError('');
    try {
      const todo = await apiPost('/api/project/todos', { title: values.title.trim(), draft: values.draft || '', initiative_id: values.initiative_id || null, board_status: values.board_status, tag_ids: values.tag_ids || [], owner_user_name: values.owner_user_name ?? null, follower_user_names: values.follower_user_names || [] });
      form.resetFields(); onNotice(ok('TODO 已创建')); await onCreated(todo.id);
    } catch (error) { setError(error.message); onNotice(err(error.message)); } finally { setSaving(false); }
  };
  return <Drawer open={open} title="新建 TODO" onClose={() => { if (!saving) onClose(); }} size="min(760px, 100vw)" destroyOnHidden
    footer={<TodoActions>{error && <Alert type="error" showIcon title={error} />}<Button disabled={saving} onClick={onClose}>取消</Button><EditButton type="primary" loading={saving} onClick={() => form.submit()}>创建</EditButton></TodoActions>}>
    <Form form={form} layout="vertical" className="project-todo-form" disabled={!canEdit || saving} onFinish={create} scrollToFirstError initialValues={{ initiative_id: initiativeId || undefined, board_status: 'todo', tag_ids: [] }}>
      <TodoSection title="任务内容">
        <Form.Item name="title" label="标题" className="project-todo-field-full" rules={[{ required: true, whitespace: true, message: '请输入标题' }]}><Input placeholder="简明描述要完成的任务" /></Form.Item>
        <Form.Item name="draft" label="任务说明" className="project-todo-field-full"><Input.TextArea autoSize={{ minRows: 6, maxRows: 18 }} placeholder="写下任务要求、交付内容和验收标准" /></Form.Item>
      </TodoSection>
      <TodoSection title="归属与状态">
        <Form.Item name="initiative_id" label="所属专项"><Select {...searchSelect} aria-label="所属专项" placeholder="未归属专项" options={groupOptions(overview)} onChange={() => form.setFieldValue('tag_ids', [])} /></Form.Item>
        <Form.Item name="board_status" label="状态"><Select options={LANES.map(([value, label]) => ({ value, label }))} /></Form.Item>
        <Form.Item name="tag_ids" label="Tag" className="project-todo-field-full"><Select mode="multiple" showSearch optionFilterProp="label" placeholder="选择标签" options={tags.map((tag) => ({ value: tag.id, label: tag.name }))} /></Form.Item>
      </TodoSection>
      <TodoSection title="负责人和关注人"><PeopleFields /></TodoSection>
    </Form>
  </Drawer>;
}
