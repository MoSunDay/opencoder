import { useEvent } from '../ui/editing/useEvent.js';
import { Button, Form, Input, Modal, Select, Space, Spin, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPut } from '../api.js';
import { err } from '../notice.js';
import { useMessage } from '../ui/appMessage.js';
import { wrapForm, wrapSettings } from './configuration.js';

const NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$/;
const ENV_RULE = [{ validator: (_, value) => { try { wrapSettings({ envs: value }); return Promise.resolve(); } catch (e) { return Promise.reject(e); } } }];

export function HarnessManagement({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [form] = Form.useForm();
  const [createForm] = Form.useForm();
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(true);
  const [ready, setReady] = useState(false);
  const [saving, setSaving] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [revision, setRevision] = useState(null);
  const [profiles, setProfiles] = useState([]);
  const [selected, setSelected] = useState('');
  const [defaults, setDefaults] = useState(null);
  const select = (name, items = profiles, fallback = defaults) => {
    const entry = name ? items.find((p) => p.name === name) : fallback;
    setDirty(false); setSelected(name); setRevision(entry?.revision ?? null);
    form.resetFields();
    form.setFieldsValue(wrapForm(entry?.settings));
  };
  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await apiGet('/api/harnesses');
      const codex = data.harnesses.find((h) => h.name === 'codex');
      if (!codex) throw new Error('Codex 配置缺失');
      setDefaults(codex); setProfiles(data.profiles || []); setSelected('');
      form.setFieldsValue(wrapForm(codex.settings));
      setRevision(codex.revision); setReady(true);
    } catch (e) { setReady(false); onNotice(err(`读取 Harness 配置失败：${e.message}`)); }
    finally { setLoading(false); }
  }, [form, onNotice]);
  useEffect(() => { load(); }, [load]);
  useEffect(() => { if (createOpen) createForm.resetFields(); }, [createForm, createOpen]);
  const save = async (values) => {
    setSaving(true);
    try {
      const settings = wrapSettings(values);
      const result = await apiPut(selected ? `/api/harnesses/codex/profiles/${encodeURIComponent(selected)}` : '/api/harnesses/codex', settings);
      const entry = { name: selected, settings, revision: result.revision };
      if (selected) setProfiles((rows) => rows.filter((row) => row.name !== selected).concat(entry));
      else setDefaults(entry);
      setDirty(false); setRevision(result.revision); msg.success('Codex 配置已保存，新任务将使用此版本');
    } catch (e) { onNotice(err(`保存 Harness 配置失败：${e.message}`)); }
    finally { setSaving(false); }
  };
  // 新建档案走后端 upsert（PUT /api/harnesses/codex/profiles/:name）：成功后
  // 刷新列表并选中新档案，失败保留表单可重试；取消即销毁表单内容。
  const createProfile = async (values) => {
    setCreating(true);
    try {
      const name = values.name.trim();
      const settings = wrapSettings(values);
      const result = await apiPut(`/api/harnesses/codex/profiles/${encodeURIComponent(name)}`, settings);
      const entry = { name, settings, revision: result.revision };
      const items = profiles.filter((row) => row.name !== name).concat(entry);
      setProfiles(items); setCreateOpen(false); select(name, items);
      msg.success(`配置档案 ${name} 已创建，新任务将使用此版本`);
    } catch (e) { onNotice(err(`创建配置档案失败：${e.message}`)); }
    finally { setCreating(false); }
  };
  return <Spin spinning={loading}>
    <Space wrap style={{ marginBottom: 16 }}>
      <Select disabled={loading || !ready || saving || dirty} aria-label="codex-profile" style={{ minWidth: 210 }} value={selected} onChange={(name) => select(name)} options={[{ value: '', label: '默认 Codex 配置' }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]} />
      <Button type="primary" disabled={loading || !ready || saving || dirty} onClick={() => setCreateOpen(true)}>新建配置档案</Button>
    </Space>
    <Space style={{ marginBottom: 16 }}><Typography.Text strong>Codex</Typography.Text><Tag>{revision ? `配置 v${revision}` : '尚未保存统一配置'}</Tag><Button disabled={loading || saving || dirty} onClick={load}>刷新配置</Button></Space>
    <Form form={form} layout="vertical" onValuesChange={() => setDirty(true)} onFinish={save} disabled={loading || !ready || saving} style={{ maxWidth: 760 }}>
      <Form.Item name="model" label="模型（--model）"><Input placeholder="留空使用 Codex 默认模型" /></Form.Item>
      <Form.Item name="envs" label="环境变量（--envs）" extra="每行一个 KEY=VALUE。保存在私有配置中，不进入 NFS 资源，也不在执行详情中显示值。" rules={ENV_RULE}>
        <Input.TextArea rows={6} aria-label="codex-managed-envs" autoComplete="off" spellCheck={false} />
      </Form.Item>
      <Button type="primary" htmlType="submit" loading={saving}>保存 Codex 配置</Button>
    </Form>
    <Modal
      title="新建配置档案"
      open={createOpen}
      okText="创建"
      cancelText="取消"
      confirmLoading={creating}
      destroyOnHidden
      onOk={() => createForm.submit()}
      onCancel={() => { if (!creating) setCreateOpen(false); }}
    >
      <Form form={createForm} layout="vertical" onFinish={createProfile} disabled={creating}>
        <Form.Item name="name" label="名称" rules={[
          { required: true, message: '请输入配置档案名称' },
          { pattern: NAME_PATTERN, message: '名称仅限字母、数字与 . _ -，以字母或数字开头，最长 48 字符' },
          { validator: (_, value) => profiles.some((p) => p.name === value?.trim()) ? Promise.reject(new Error('配置档案名称已存在')) : Promise.resolve() },
        ]}>
          <Input placeholder="如 codex-review" aria-label="new-codex-profile" autoComplete="off" />
        </Form.Item>
        <Form.Item name="model" label="模型（--model）" initialValue=""><Input placeholder="留空使用 Codex 默认模型" aria-label="new-codex-profile-model" /></Form.Item>
        <Form.Item name="envs" label="环境变量（--envs）" extra="每行一个 KEY=VALUE。保存在私有配置中，不进入 NFS 资源，也不在执行详情中显示值。" initialValue="" rules={ENV_RULE}>
          <Input.TextArea rows={6} aria-label="new-codex-profile-envs" autoComplete="off" spellCheck={false} />
        </Form.Item>
      </Form>
    </Modal>
  </Spin>;
}
