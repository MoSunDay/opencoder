import { useEvent } from '../ui/editing/useEvent.js';
import { Alert, Button, Form, Input, Select, Space, Spin, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPut } from '../api.js';
import { err } from '../notice.js';
import { useMessage } from '../ui/appMessage.js';
import { wrapForm, wrapSettings } from './configuration.js';

export function HarnessManagement({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const msg = useMessage();
  const [form] = Form.useForm();
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(true);
  const [ready, setReady] = useState(false);
  const [saving, setSaving] = useState(false);
  const [revision, setRevision] = useState(null);
  const [profiles, setProfiles] = useState([]);
  const [selected, setSelected] = useState('');
  const [newName, setNewName] = useState('');
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
  return <Spin spinning={loading}>
    <Space wrap style={{ marginBottom: 16 }}>
      <Select disabled={loading || !ready || saving || dirty} aria-label="codex-profile" style={{ minWidth: 210 }} value={selected} onChange={(name) => select(name)} options={[{ value: '', label: '默认 Codex 配置' }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]} />
      <Input disabled={loading || !ready || saving || dirty} aria-label="new-codex-profile" placeholder="新配置档案名称" value={newName} onChange={(e) => setNewName(e.target.value)} />
      <Button disabled={loading || !ready || saving || dirty} onClick={() => {
        if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$/.test(newName) || profiles.some((p) => p.name === newName)) { onNotice(err('请输入未使用的配置档案名称')); return; }
        const items = profiles.concat({ name: newName, settings: {} }); setProfiles(items); select(newName, items); setNewName('');
      }}>新增配置档案</Button>
    </Space>
    <Space style={{ marginBottom: 16 }}><Typography.Text strong>Codex</Typography.Text><Tag>{revision ? `配置 v${revision}` : '尚未保存统一配置'}</Tag><Button disabled={loading || saving || dirty} onClick={load}>刷新配置</Button></Space>
    <Alert type="info" showIcon title="Wrap 参数统一管理" description="配置 opencoder --wrap codex 的 --model 和 --envs 参数。Codex 使用执行节点已有的安装与自身配置；任务要求在启动时填写。" style={{ marginBottom: 16 }} />
    <Form form={form} layout="vertical" onValuesChange={() => setDirty(true)} onFinish={save} disabled={loading || !ready || saving} style={{ maxWidth: 760 }}>
      <Form.Item name="model" label="模型（--model）"><Input placeholder="留空使用 Codex 默认模型" /></Form.Item>
      <Form.Item name="envs" label="环境变量（--envs）" extra="每行一个 KEY=VALUE。保存在私有配置中，不进入 NFS 资源，也不在执行详情中显示值。" rules={[{ validator: (_, value) => { try { wrapSettings({ envs: value }); return Promise.resolve(); } catch (e) { return Promise.reject(e); } } }]}>
        <Input.TextArea rows={6} aria-label="codex-managed-envs" autoComplete="off" spellCheck={false} />
      </Form.Item>
      <Button type="primary" htmlType="submit" loading={saving}>保存 Codex 配置</Button>
    </Form>
  </Spin>;
}
