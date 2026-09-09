import { useEvent } from '../ui/editing/useEvent.js';
import { Alert, Button, Form, Input, InputNumber, Select, Space, Spin, Table, Tag, Typography, message } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPut } from '../api.js';
import { err } from '../notice.js';
import { HARNESS_OPTIONS, parseEnvs } from './fields.jsx';

const choices = (values) => values.map((value) => ({ value, label: value }));
const optional = (value) => value?.trim() || null;

export function HarnessManagement({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
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
    form.setFieldsValue({ ...entry?.settings, envs: Object.entries(entry?.settings?.envs || {}).map(([k, v]) => `${k}=${v}`).join('\n') });
  };
  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await apiGet('/api/harnesses');
      const codex = data.harnesses.find((h) => h.name === 'codex');
      if (!codex) throw new Error('Codex 配置缺失');
      setDefaults(codex); setProfiles(data.profiles || []); setSelected('');
      form.setFieldsValue({ ...codex.settings, envs: Object.entries(codex.settings.envs || {}).map(([k, v]) => `${k}=${v}`).join('\n') });
      setRevision(codex.revision); setReady(true);
    } catch (e) { setReady(false); onNotice(err(`读取 Harness 配置失败：${e.message}`)); }
    finally { setLoading(false); }
  }, [form, onNotice]);
  useEffect(() => { load(); }, [load]);
  const save = async (values) => {
    setSaving(true);
    try {
      const settings = Object.fromEntries(['executable', 'model', 'reasoning_effort', 'sandbox_mode', 'approval_policy'].map((key) => [key, optional(values[key])]));
      settings.envs = parseEnvs(values.envs);
      settings.auth_slot = values.auth_slot || null;
      const result = await apiPut(selected ? `/api/harnesses/codex/profiles/${encodeURIComponent(selected)}` : '/api/harnesses/codex', settings);
      const entry = { name: selected, settings, revision: result.revision };
      if (selected) setProfiles((rows) => rows.filter((row) => row.name !== selected).concat(entry));
      else setDefaults(entry);
      setDirty(false); setRevision(result.revision); message.success('Codex 配置已保存，新任务将使用此版本');
    } catch (e) { onNotice(err(`保存 Harness 配置失败：${e.message}`)); }
    finally { setSaving(false); }
  };
  return <Spin spinning={loading}>
    <Space wrap style={{ marginBottom: 16 }}>
      <Select disabled={loading || saving} aria-label="codex-profile" style={{ minWidth: 210 }} value={selected} onChange={(name) => select(name)} options={[{ value: '', label: '默认 Codex 配置' }, ...profiles.map((p) => ({ value: p.name, label: p.name }))]} />
      <Input disabled={loading || saving} aria-label="new-codex-profile" placeholder="新配置档案名称" value={newName} onChange={(e) => setNewName(e.target.value)} />
      <Button disabled={loading || saving} onClick={() => {
        if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$/.test(newName) || profiles.some((p) => p.name === newName)) { onNotice(err('请输入未使用的配置档案名称')); return; }
        const items = profiles.concat({ name: newName, settings: {} }); setProfiles(items); select(newName, items); setNewName('');
      }}>新增配置档案</Button>
    </Space>
    <Space style={{ marginBottom: 16 }}><Typography.Text strong>Codex</Typography.Text><Tag>{revision ? `配置 v${revision}` : '尚未保存统一配置'}</Tag><Button disabled={loading || saving || dirty} onClick={load}>刷新配置</Button></Space>
    <Alert type="info" showIcon title="Wrap 参数统一管理" description="Codex 使用这里的模型、权限策略和环境变量。新任务接受时固定配置，排队、续聊和恢复不随之后的修改变化；OpenCoder 原生模型参数不参与 Codex 执行。" style={{ marginBottom: 16 }} />
    <Form form={form} layout="vertical" onValuesChange={() => setDirty(true)} onFinish={save} disabled={loading || !ready || saving} style={{ maxWidth: 760 }}>
      <Form.Item name="executable" label="Codex 二进制路径" extra="执行节点上的前台入口；留空时从节点 PATH 查找 codex。"><Input autoComplete="off" placeholder="/usr/local/libexec/opencoder-harness/bin/codex" /></Form.Item>
      <Form.Item name="model" label="Codex 模型"><Input placeholder="留空使用 Codex 默认模型" /></Form.Item>
      <Form.Item name="auth_slot" label="授权槽位" extra="绑定执行节点已有的 Codex 账号槽位。"><InputNumber min={1} precision={0} aria-label="codex-auth-slot" /></Form.Item>
      <Form.Item name="reasoning_effort" label="推理强度"><Select allowClear placeholder="Codex 默认值" options={choices(['none', 'minimal', 'low', 'medium', 'high', 'xhigh'])} /></Form.Item>
      <Form.Item name="sandbox_mode" label="沙箱权限"><Select allowClear placeholder="Codex 默认值" options={choices(['read-only', 'workspace-write', 'danger-full-access'])} /></Form.Item>
      <Form.Item name="approval_policy" label="审批策略" extra="无人值守执行建议配置 never；需要交互审批的命令可能无法执行。"><Select allowClear placeholder="Codex 默认值" options={choices(['never', 'on-request', 'untrusted'])} /></Form.Item>
      <Form.Item name="envs" label="Codex 环境变量" extra="每行一个 KEY=VALUE。保存在私有配置中，不进入 NFS 资源，也不在执行详情中显示值。" rules={[{ validator: (_, value) => { try { parseEnvs(value); return Promise.resolve(); } catch (e) { return Promise.reject(e); } } }]}>
        <Input.TextArea rows={6} aria-label="codex-managed-envs" autoComplete="off" spellCheck={false} />
      </Form.Item>
      <Button type="primary" htmlType="submit" loading={saving}>保存 Codex 配置</Button>
    </Form>
  </Spin>;
}

export function AgentHarnessSettings({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const [profiles, setProfiles] = useState([]);
  const [agents, setAgents] = useState([]); const [loading, setLoading] = useState(false); const [saving, setSaving] = useState(null);
  const load = useCallback(async () => {
    setLoading(true);
    try { const [data, harnesses] = await Promise.all([apiGet('/api/agents'), apiGet('/api/harnesses')]); setAgents(data.agents); setProfiles(harnesses.profiles || []); }
    catch (e) { onNotice(err(`读取 Agent 失败：${e.message}`)); }
    finally { setLoading(false); }
  }, [onNotice]);
  useEffect(() => { load(); }, [load]);
  const save = async (name, harness, profile) => {
    setSaving(name);
    try { await apiPut(`/api/agents/${encodeURIComponent(name)}`, { harness, ...(profile !== undefined ? { harness_profile: profile } : {}) }); await load(); message.success('Agent Harness 已更新，仅影响新任务'); }
    catch (e) { onNotice(err(e.message)); }
    finally { setSaving(null); }
  };
  return <>
    <Alert type="info" showIcon title="选择 Agent 的执行方式" description="所有内置与自定义 Agent 均可选择 OpenCoder 或 Codex。Codex 参数在「Harness 管理」中统一设置，运行中的任务保持原配置。" style={{ marginBottom: 16 }} />
    <Table rowKey="name" loading={loading} dataSource={agents} pagination={false} columns={[
      { title: 'Agent', dataIndex: 'name' },
      { title: '类型', render: (_, row) => row.builtin ? '内置' : '自定义' },
      { title: 'Harness', render: (_, row) => <Select aria-label={`harness-${row.name}`} style={{ minWidth: 160 }} value={row.harness || 'opencoder'} options={HARNESS_OPTIONS} disabled={saving === row.name} onChange={(harness) => save(row.name, harness)} /> },
      { title: '参数来源', render: (_, row) => row.harness === 'codex' ? <Select aria-label={`profile-${row.name}`} style={{ minWidth: 180 }} value={row.harness_profile || ''} disabled={saving === row.name} options={[{ value: '', label: '默认 Codex 配置' }, ...profiles.map((p) => ({ value: p.name, label: `${p.name} · v${p.revision}` }))]} onChange={(value) => save(row.name, 'codex', value || null)} /> : 'OpenCoder 原生配置' },
    ]} />
  </>;
}
