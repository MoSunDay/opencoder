// OpenCoder Env 管理：每个 Env 是一套完整配置快照（config、MCP、CLI、Skills、Autopilot）。
import { Button, Drawer, Form, Input, Modal, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from './api.js';
import { PageShell } from './shell/pageShell.jsx';
import { err } from './notice.js';
import { useEvent } from './ui/editing/useEvent.js';
import { useMessage } from './ui/appMessage.js';
import { tableLoading, tableRows } from './ui/tableLoading.js';

const { Text } = Typography;
const FILE_KEYS = ['config', 'mcp_servers', 'cli', 'skills', 'autopilot'];
const LABELS = { config: 'opencoder.json', mcp_servers: 'MCP（mcp.json）', cli: 'CLI（cli.json）', skills: 'Skills（skills.json）', autopilot: 'Autopilot（ap.json）' };

function CreateEnvModal({ open, onClose, onCreated, onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback); const msg = useMessage(); const [form] = Form.useForm(); const [saving, setSaving] = useState(false);
  const submit = async (values) => { setSaving(true); try { await apiPost('/api/envs', { name: values.name, capture_current: values.capture_current !== false }); msg.success('已创建'); form.resetFields(); onCreated(values.name); } catch (e) { onNotice(err('新建 env 失败: ' + (e?.message || e))); } finally { setSaving(false); } };
  return <Modal open={open} title="新建 OpenCoder Env" onCancel={() => !saving && onClose()} footer={null} destroyOnHidden>
    <Form form={form} layout="vertical" onFinish={submit} initialValues={{ capture_current: true }} disabled={saving}>
      <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}><Input placeholder="production" /></Form.Item>
      <Form.Item name="capture_current" label="初始化方式"><Select options={[{ value: true, label: '复制当前 OpenCoder 配置' }, { value: false, label: '创建空配置集' }]} /></Form.Item>
      <Space><Button type="primary" htmlType="submit" loading={saving}>创建</Button><Button onClick={onClose}>取消</Button></Space>
    </Form>
  </Modal>;
}

function JsonEditor({ value, onChange, disabled }) {
  return <Input.TextArea value={value} onChange={(e) => onChange(e.target.value)} disabled={disabled} autoComplete="off" spellCheck={false} style={{ fontFamily: 'monospace', minHeight: 360 }} />;
}

function EnvDrawer({ name, open, onClose, onNotice: noticeCallback, onSaved }) {
  const onNotice = useEvent(noticeCallback); const msg = useMessage(); const [files, setFiles] = useState({}); const [active, setActive] = useState('config'); const [loading, setLoading] = useState(false); const [saving, setSaving] = useState(false);
  const load = useCallback(async () => { if (!open || !name) return; setLoading(true); try { const j = await apiGet(`/api/envs/${encodeURIComponent(name)}`); const incoming = {}; FILE_KEYS.forEach((k) => { incoming[k] = JSON.stringify(j?.files?.[k] || {}, null, 2); }); setFiles(incoming); } catch (e) { onNotice(err('获取 env 配置失败: ' + (e?.message || e))); } finally { setLoading(false); } }, [open, name, onNotice]);
  useEffect(() => { load(); }, [load]);
  const save = async () => { if (saving || loading) return; const parsed = {}; try { FILE_KEYS.forEach((k) => { parsed[k] = JSON.parse(files[k] || '{}'); }); } catch (e) { onNotice(err('配置 JSON 格式错误: ' + (e?.message || e))); return; } setSaving(true); try { await apiPatch(`/api/envs/${encodeURIComponent(name)}`, { files: parsed }); msg.success('Env 配置已保存'); onSaved?.(); } catch (e) { onNotice(err('保存 env 配置失败: ' + (e?.message || e))); } finally { setSaving(false); } };
  return <Drawer title={`编辑 OpenCoder Env: ${name || ''}`} open={open} onClose={() => !saving && onClose()} width={760} destroyOnHidden footer={<Space style={{ float: 'right' }}><Button onClick={onClose} disabled={saving}>关闭</Button><Button type="primary" onClick={save} loading={saving} disabled={loading}>保存整套配置</Button></Space>}>
    <Select value={active} onChange={setActive} options={FILE_KEYS.map((k) => ({ value: k, label: LABELS[k] }))} style={{ width: '100%', marginBottom: 12 }} disabled={loading || saving} />
    <JsonEditor value={files[active] || '{}'} onChange={(v) => setFiles((prev) => ({ ...prev, [active]: v }))} disabled={loading || saving} />
    <Typography.Paragraph type="secondary" style={{ marginTop: 8 }}>保存会更新该 Env 的完整 OpenCoder 配置；当前 Env 激活时会立即通知运行中的会话重新加载。api_key 的掩码（***）会保留原凭据。</Typography.Paragraph>
  </Drawer>;
}

export function EnvsPanel({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback); const msg = useMessage(); const [envs, setEnvs] = useState([]); const [active, setActive] = useState(null); const [loading, setLoading] = useState(true); const [creating, setCreating] = useState(false); const [editing, setEditing] = useState('');
  const load = useCallback(async (silent = false) => { if (!silent) setLoading(true); try { const j = await apiGet('/api/envs'); setEnvs(j?.envs || []); setActive(j?.active || null); } catch (e) { onNotice(err('获取 OpenCoder env 列表失败: ' + (e?.message || e))); } finally { if (!silent) setLoading(false); } }, [onNotice]);
  useEffect(() => { load(); }, [load]);
  const activate = async (name) => { try { await apiPatch('/api/envs', { active: name }); msg.success(`已切换到 ${name}`); load(true); } catch (e) { onNotice(err('切换 env 失败: ' + (e?.message || e))); } };
  const deleteEnv = async (name) => { try { await apiDel(`/api/envs/${encodeURIComponent(name)}`); msg.success('已删除'); if (editing === name) setEditing(''); load(true); } catch (e) { onNotice(err('删除 env 失败: ' + (e?.message || e))); } };
  const columns = [
    { title: '名称', dataIndex: 'name', key: 'name', render: (v) => <Text strong>{v}</Text> },
    { title: '状态', key: 'active', width: 100, render: (_, e) => e.active || e.name === active ? <Tag color="green">当前</Tag> : <Tag>未激活</Tag> },
    { title: '操作', key: 'ops', width: 220, render: (_, e) => <Space size={0}><Button size="small" type="link" onClick={() => setEditing(e.name)}>编辑配置</Button>{e.name === active ? <Button size="small" type="link" onClick={() => activate(null)}>停用</Button> : <Button size="small" type="link" onClick={() => activate(e.name)}>激活</Button>}<Popconfirm title={`删除 env ${e.name}？`} okText="确认删除" onConfirm={() => deleteEnv(e.name)}><Button size="small" danger type="link">删除</Button></Popconfirm></Space> },
  ];
  return <PageShell page="envs" extra={<Button type="primary" onClick={() => setCreating(true)}>新建 Env</Button>}>
    <Table rowKey="name" size="small" columns={columns} dataSource={tableRows(loading, envs)} loading={tableLoading(loading)} pagination={false} locale={{ emptyText: '暂无 OpenCoder Env' }} />
    <CreateEnvModal open={creating} onClose={() => setCreating(false)} onNotice={onNotice} onCreated={(name) => { setCreating(false); setEditing(name); load(true); }} />
    <EnvDrawer name={editing} open={!!editing} onClose={() => setEditing('')} onNotice={onNotice} onSaved={() => load(true)} />
  </PageShell>;
}
