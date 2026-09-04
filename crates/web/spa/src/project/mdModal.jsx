// mdModal.jsx — shared 新建/编辑 modal for goals & milestones: 标题 + 排序 +
// Markdown 详情 with a live 编辑/预览 Segmented switch. Purely presentational:
// the caller seeds `initial` ({title, sort, detail_md}) and receives the final
// value via onOk — the POST-vs-PATCH decision stays in the owning tab. An
// optional `extraTop` node (e.g. the milestone tab's goal Select) renders
// above 标题 and stays caller-owned.

import { Button, Form, Input, InputNumber, Modal, Segmented } from 'antd';
import { useEffect, useState } from 'react';
import { Markdown } from './markdown.jsx';

const { TextArea } = Input;

export function MdEditModal({ open, title, initial, extraTop, onCancel, onOk }) {
  const [form] = Form.useForm();
  const [mode, setMode] = useState('edit');
  const [saving, setSaving] = useState(false);
  const detail = Form.useWatch('detail_md', form) || '';

  // Re-seed on every open (create ⇒ blank, edit ⇒ record fields).
  useEffect(() => {
    if (open) {
      form.resetFields();
      form.setFieldsValue({
        title: (initial && initial.title) || '',
        sort: (initial && initial.sort) || 0,
        detail_md: (initial && initial.detail_md) || '',
      });
      setMode('edit');
      setSaving(false);
    }
  }, [open, initial, form]);

  const submit = async () => {
    let values;
    try {
      values = await form.validateFields();
    } catch {
      return; // antd already pinned the required-field messages
    }
    setSaving(true);
    try {
      await onOk({
        title: values.title,
        sort: Number.isFinite(values.sort) ? values.sort : 0,
        detail_md: values.detail_md || '',
      });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={title}
      onCancel={onCancel}
      destroyOnHidden
      footer={[
        <Button key="cancel" onClick={onCancel}>取消</Button>,
        <Button key="ok" type="primary" loading={saving} onClick={submit}>保存</Button>,
      ]}
    >
      <Form form={form} layout="vertical" preserve={false}>
        <Form.Item name="title" label="标题" rules={[{ required: true, message: '请输入标题' }]}>
          <Input placeholder="一句话标题" />
        </Form.Item>
        <Form.Item name="sort" label="排序" tooltip="数字小的排在前面">
          <InputNumber style={{ width: 140 }} />
        </Form.Item>
        <Form.Item label="详情（Markdown）">
          <Segmented
            value={mode}
            onChange={(v) => setMode(v)}
            options={[{ label: '编辑', value: 'edit' }, { label: '预览', value: 'preview' }]}
            style={{ marginBottom: 8 }}
          />
        </Form.Item>
        {mode === 'edit' ? (
          <Form.Item name="detail_md" noStyle>
            <TextArea rows={6} placeholder="支持 Markdown：# 标题、列表、代码块…" aria-label="detail_md" />
          </Form.Item>
        ) : (
          <div
            className="md-modal-preview"
            style={{ minHeight: 140, padding: 12, border: '1px solid #f0f0f0', borderRadius: 6 }}
            aria-label="detail_preview"
          >
            <Markdown text={detail} />
          </div>
        )}
      </Form>
    </Modal>
  );
}
