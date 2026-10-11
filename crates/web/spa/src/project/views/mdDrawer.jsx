import { EditButton } from '../../ui/permissions.jsx';
import { Button, Drawer, Form, Input, InputNumber, Segmented, Space } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { Markdown } from '../markdown.jsx';
import { PeopleFields } from '../people/fields.jsx';

export function MdEditDrawer({ open, title, initial, extraTop, onCancel, onOk }) {
  const [form] = Form.useForm();
  const [mode, setMode] = useState('edit');
  const [saving, setSaving] = useState(false);
  const seed = useRef(initial);
  const initialSummary = useRef('');
  const submitting = useRef(false);
  seed.current = initial;
  const recordId = initial?.id || 'new';

  useEffect(() => {
    if (!open) return;
    initialSummary.current = seed.current?.current_summary || '';
    form.resetFields();
    form.setFieldsValue({
      title: seed.current?.title || '',
      sort: seed.current?.sort || 0,
      detail_md: seed.current?.detail_md || '',
      current_summary: seed.current?.current_summary || '',
      goal_id: seed.current?.goal_id ?? null,
      owner_user_name: seed.current?.owner_user_name ?? null,
      follower_user_names: seed.current?.follower_user_names || [],
    });
    setMode('edit');
  }, [open, recordId, form]);

  const submit = async () => {
    if (submitting.current) return;
    let values;
    try { values = await form.validateFields(); } catch { return; }
    submitting.current = true;
    setSaving(true);
    try {
      const { current_summary, ...fields } = values;
      const summary = current_summary === initialSummary.current ? {} : { current_summary };
      await onOk({ ...fields, ...summary, sort: Number.isFinite(values.sort) ? values.sort : 0, detail_md: values.detail_md || '' });
    } finally {
      submitting.current = false;
      setSaving(false);
    }
  };

  return <Drawer open={open} title={title} onClose={() => { if (!saving) onCancel(); }} size="min(640px, 100vw)" destroyOnHidden
    extra={<Space><Button disabled={saving} onClick={onCancel}>取消</Button><EditButton type="primary" loading={saving} onClick={submit}>保存</EditButton></Space>}>
    <Form form={form} layout="vertical" disabled={saving}>
      {extraTop}
      <PeopleFields />
      <Form.Item name="title" label="标题" rules={[{ required: true, message: '请输入标题' }]}><Input placeholder="一句话标题" /></Form.Item>
      <Form.Item name="current_summary" label="当前概述" extra="记录关键进展、核心工作和剩余卡点。"><Input.TextArea rows={6} aria-label="current_summary" /></Form.Item>
      <Form.Item name="sort" label="排序"><InputNumber style={{ width: 140 }} /></Form.Item>
      <Form.Item label="详情（Markdown）"><Segmented value={mode} onChange={setMode} options={[
        { label: '编辑', value: 'edit' }, { label: '预览', value: 'preview' },
      ]} /></Form.Item>
      <div hidden={mode !== 'edit'}><Form.Item name="detail_md" noStyle><Input.TextArea rows={8} aria-label="detail_md" /></Form.Item></div>
      {mode === 'preview' && <div aria-label="detail_preview" style={{ minHeight: 140, padding: 12 }}>
        <Form.Item noStyle shouldUpdate>{() => <Markdown text={form.getFieldValue('detail_md')} />}</Form.Item>
      </div>}
    </Form>
  </Drawer>;
}
