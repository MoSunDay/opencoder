import { EditButton } from '../../ui/permissions.jsx';
import { Alert, Button, Form, Input, Modal, Space } from 'antd';
import { useState } from 'react';
import { apiPost, apiPut } from '../../api.js';
import { encodeUpload, MAX_BINARY_BYTES } from './model.js';

export function BinaryEditor({ pool, onClose, onSaved }) {
  const [form] = Form.useForm();
  const [file, setFile] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const save = async (values) => {
    if (busy) return;
    setError('');
    if (!file || file.size > MAX_BINARY_BYTES) { setError('请选择不超过 32 MiB 的 Linux ELF 可执行文件'); return; }
    setBusy(true);
    try {
      const body = { description: values.description || '', binary_b64: encodeUpload(await file.arrayBuffer()) };
      if (pool) await apiPut(`/api/dag/binaries/${encodeURIComponent(pool.name)}`, body);
      else await apiPost('/api/dag/binaries', { ...body, name: values.name.trim() });
      await onSaved();
      onClose();
    } catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  return <Modal open title={pool ? `追加二进制版本 · ${pool.name}` : '上传二进制资源'} onCancel={() => { if (!busy) onClose(); }} footer={null} destroyOnHidden>
    <Form form={form} layout="vertical" initialValues={{ name: pool?.name, description: pool?.description }} onFinish={save}>
      {error && <Alert type="error" showIcon title={error} />}
      <Form.Item name="name" label="资源名称" rules={[{ required: true, message: '请输入资源名称' }, { pattern: /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,47}$/, message: '使用字母、数字、下划线、点或连字符，最多 48 字符，不能以点开头' }]}>
        <Input aria-label="二进制资源名称" disabled={!!pool || busy} />
      </Form.Item>
      <Form.Item name="description" label="说明"><Input.TextArea aria-label="二进制资源说明" disabled={busy} rows={2} /></Form.Item>
      <Form.Item label="Linux 可执行文件" extra="ELF64，x86_64 或 aarch64，不超过 32 MiB；新版本不会改写已受理任务的固定版本。">
        <input type="file" aria-label="Linux 可执行文件" disabled={busy} onChange={(event) => { setFile(event.target.files?.[0] || null); setError(''); }} />
      </Form.Item>
      <Space><EditButton type="primary" htmlType="submit" loading={busy}>保存二进制</EditButton><Button disabled={busy} onClick={onClose}>取消</Button></Space>
    </Form>
  </Modal>;
}
