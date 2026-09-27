import { Button, Drawer, Form, Input, InputNumber, Segmented, Space } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { Markdown } from '../markdown.jsx';

export function MdEditDrawer({ open, title, initial, extraTop, onCancel, onOk }) {
  const [form] = Form.useForm();
  const [mode, setMode] = useState('edit');
  const [saving, setSaving] = useState(false);
  const seed = useRef(initial);
  const submitting = useRef(false);
  seed.current = initial;
  const recordId = initial?.id || 'new';

  useEffect(() => {
    if (!open) return;
    form.resetFields();
    form.setFieldsValue({
      title: seed.current?.title || '',
      sort: seed.current?.sort || 0,
      detail_md: seed.current?.detail_md || '',
      goal_id: seed.current?.goal_id ?? null,
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
      await onOk({ ...values, sort: Number.isFinite(values.sort) ? values.sort : 0, detail_md: values.detail_md || '' });
    } finally {
      submitting.current = false;
      setSaving(false);
    }
  };

  return <Drawer open={open} title={title} onClose={() => { if (!saving) onCancel(); }} size={640} destroyOnHidden
    extra={<Space><Button disabled={saving} onClick={onCancel}>取消</Button><Button type="primary" loading={saving} onClick={submit}>保存</Button></Space>}>
    <Form form={form} layout="vertical" disabled={saving}>
      {extraTop}
      <Form.Item name="title" label="标题" rules={[{ required: true, message: '请输入标题' }]}><Input placeholder="一句话标题" /></Form.Item>
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
