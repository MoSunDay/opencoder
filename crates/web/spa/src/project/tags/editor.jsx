import { Button, Drawer, Form, Input, Select, Space } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { initiativeOptions, projectOptions, searchSelect } from '../model/relations.js';

export function TagEditor({ open, initial, overview, onCancel, onOk }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const submitting = useRef(false);
  const seed = useRef(initial);
  seed.current = initial;
  const recordId = initial?.id || 'new';
  const scopeType = Form.useWatch('scope_type', form) ?? initial?.scope_type ?? 'project';
  const owners = scopeType === 'initiative' ? initiativeOptions(overview) : projectOptions(overview);

  useEffect(() => {
    if (!open) return;
    form.resetFields();
    form.setFieldsValue({ name: seed.current?.name || '', scope_type: seed.current?.scope_type || 'project', scope_id: seed.current?.scope_id });
  }, [open, recordId, form]);

  const submit = async () => {
    if (submitting.current) return;
    submitting.current = true;
    try {
      let values;
      try { values = await form.validateFields(); } catch { return; }
      setSaving(true);
      await onOk({ name: values.name.trim(), scope_type: values.scope_type, scope_id: values.scope_id });
    } finally {
      submitting.current = false;
      setSaving(false);
    }
  };

  return <Drawer open={open} title={initial ? '编辑 Tag' : '新建 Tag'} size="min(640px, 100vw)" destroyOnHidden
    onClose={() => { if (!submitting.current) onCancel(); }}
    extra={<Space><Button disabled={saving} onClick={onCancel}>取消</Button><Button type="primary" loading={saving} onClick={submit}>保存 Tag</Button></Space>}>
    <Form form={form} layout="vertical" disabled={saving} onValuesChange={(changed) => {
      if (Object.hasOwn(changed, 'scope_type')) form.setFields([{ name: 'scope_id', value: undefined, errors: [] }]);
    }}>
      <Form.Item name="name" label="Tag 名称" rules={[{ validator: (_, value) => {
        const length = Array.from((value || '').trim()).length;
        return length >= 1 && length <= 128 ? Promise.resolve() : Promise.reject(new Error('请输入 1–128 字符的 Tag 名称'));
      } }]}><Input aria-label="Tag 名称" placeholder="输入 Tag 名称" /></Form.Item>
      <Form.Item name="scope_type" label="归属类型" rules={[{ required: true }]}>
        <Select aria-label="Tag 归属类型" options={[{ value: 'project', label: '项目' }, { value: 'initiative', label: '专项' }]} />
      </Form.Item>
      <Form.Item name="scope_id" label={scopeType === 'initiative' ? '所属专项' : '所属项目'} rules={[{ required: true, message: '请选择具体项目或专项' }, {
        validator: (_, value) => !value || owners.some((owner) => owner.value === value) ? Promise.resolve() : Promise.reject(new Error('归属已删除，请重新选择')),
      }]}><Select {...searchSelect} aria-label="Tag 归属" options={owners} placeholder="选择具体项目或专项" /></Form.Item>
    </Form>
  </Drawer>;
}
