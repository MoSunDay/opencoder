import { EditButton } from '../../ui/permissions.jsx';
import { Button, Drawer, Form, Input, InputNumber, Space } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { MarkdownInput } from '../forms/markdownInput.jsx';
import { useCloseGuard } from '../forms/closeGuard.js';
import { PeopleFields } from '../people/fields.jsx';

export function MdEditDrawer({ open, title, initial, extraTop, detailLabel = '项目目标', onCancel, onOk }) {
  const [form] = Form.useForm();
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const seed = useRef(initial);
  const initialSummary = useRef('');
  const submitting = useRef(false);
  seed.current = initial;
  const recordId = initial?.id || 'new';
  const [close, holder] = useCloseGuard({ dirty, busy: saving, onClose: onCancel, active: open });

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
    setDirty(false);
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
      if (await onOk({ ...fields, ...summary, sort: Number.isFinite(values.sort) ? values.sort : 0, detail_md: values.detail_md || '' })) setDirty(false);
    } finally {
      submitting.current = false;
      setSaving(false);
    }
  };

  return <Drawer open={open} title={title} onClose={close} size="min(640px, 100vw)" destroyOnHidden
    footer={<Space wrap><Button disabled={saving} onClick={close}>取消</Button><EditButton type="primary" loading={saving} onClick={submit}>保存</EditButton></Space>}>
    {holder}
    <Form form={form} layout="vertical" disabled={saving} onValuesChange={() => setDirty(true)}>
      <Form.Item name="title" label="标题" rules={[{ required: true, message: '请输入标题' }]}><Input placeholder="一句话标题" /></Form.Item>
      {extraTop}
      <Form.Item name="detail_md" label={`${detailLabel}（Markdown）`}><MarkdownInput label={detailLabel} /></Form.Item>
      <Form.Item name="current_summary" label="现状（Markdown）" extra="记录关键进展、核心工作和剩余卡点。"><MarkdownInput label="current_summary" /></Form.Item>
      <PeopleFields />
      <Form.Item name="sort" label="排序"><InputNumber style={{ width: 140 }} /></Form.Item>
    </Form>
  </Drawer>;
}
