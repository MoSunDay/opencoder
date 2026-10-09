import { EditButton } from '../../ui/permissions.jsx';
import { Button, Drawer, Form, Input, Select } from 'antd';
import { useState } from 'react';
import { apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';
import { groupOptions, searchSelect } from '../model/relations.js';
import { effectiveTags } from '../model/catalog.js';
import { LANES } from '../model/board.js';

export function CreateTodo({ open, overview, initiativeId, onClose, onCreated, onNotice }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const selected = Form.useWatch('initiative_id', form);
  const tags = effectiveTags(overview, selected);
  const create = async (values) => {
    if (saving) return;
    setSaving(true);
    try {
      const todo = await apiPost('/api/project/todos', { title: values.title.trim(), draft: values.draft || '', initiative_id: values.initiative_id || null, board_status: values.board_status, tag_ids: values.tag_ids || [] });
      form.resetFields(); onNotice(ok('TODO 已创建')); await onCreated(todo.id);
    } catch (error) { onNotice(err(error.message)); } finally { setSaving(false); }
  };
  return <Drawer open={open} title="新建 TODO" onClose={() => { if (!saving) onClose(); }} size="min(600px, 100vw)" destroyOnHidden
    extra={<EditButton type="primary" loading={saving} onClick={() => form.submit()}>创建</EditButton>}>
    <Form form={form} layout="vertical" disabled={saving} onFinish={create} initialValues={{ initiative_id: initiativeId || undefined, board_status: 'todo', tag_ids: [] }}>
      <Form.Item name="title" label="标题" rules={[{ required: true, whitespace: true, message: '请输入标题' }]}><Input /></Form.Item>
      <Form.Item name="initiative_id" label="所属专项"><Select {...searchSelect} aria-label="所属专项" placeholder="未归属专项" options={groupOptions(overview)} onChange={() => form.setFieldValue('tag_ids', [])} /></Form.Item>
      <Form.Item name="board_status" label="状态"><Select options={LANES.map(([value, label]) => ({ value, label }))} /></Form.Item>
      <Form.Item name="tag_ids" label="Tag"><Select mode="multiple" showSearch optionFilterProp="label" disabled={!selected} placeholder={selected ? '选择标签' : '关联专项后可选择标签'} options={tags.map((tag) => ({ value: tag.id, label: tag.name }))} /></Form.Item>
      <Form.Item name="draft" label="任务说明"><Input.TextArea rows={6} /></Form.Item>
    </Form>
  </Drawer>;
}
