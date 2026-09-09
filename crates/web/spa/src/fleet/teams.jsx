import { Alert, Button, Form, Input, Modal, Select, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { ExecutionDetail } from './detail.jsx';
import { newId, nodeOptions } from './model.js';
import { err } from '../notice.js';

export function FleetTeamsPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [nodes, setNodes] = useState([]); const [agents, setAgents] = useState([]);
  const [editing, setEditing] = useState(false); const [launch, setLaunch] = useState(null); const [detail, setDetail] = useState(null);
  const [busy, setBusy] = useState(false); const [form] = Form.useForm(); const [runForm] = Form.useForm(); const attempt = useRef(null);
  const load = useCallback(async () => {
    try {
      const [a, b, c] = await Promise.all([apiGet('/api/teams'), apiGet('/api/nodes'), apiGet('/api/agents')]);
      setRows(a.teams); setNodes(b.nodes);
      const entries = Array.isArray(c) ? c : (c.agents || []);
      setAgents(entries.map((a) => ({ value: a.name || a.id, label: a.name || a.id })));
    } catch (e) { onNotice(err(e.message)); }
  }, [onNotice]);
  useEffect(() => { load(); }, [load]);
  const edit = (row) => {
    form.setFieldsValue(row || { name: '', captain: 'captain', members: [{ id: 'captain', agent: 'act', role: '协调任务并汇总结果' }] }); setEditing(true);
  };
  const save = async (values) => {
    setBusy(true);
    try { await apiPost('/api/teams', values); onNotice(err('')); setEditing(false); await load(); }
    catch (e) { onNotice(err(e.message)); }
    finally { setBusy(false); }
  };
  const run = async (values) => {
    const request = { kind: 'team', target: launch.name, node_id: values.node || null, input: { prompt: values.prompt } };
    const signature = JSON.stringify(request);
    if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('team') };
    setBusy(true);
    try { const result = await apiPost('/api/executions', { ...request, id: attempt.current.id }); onNotice(err('')); attempt.current = null; setLaunch(null); setDetail(result); }
    catch (e) { onNotice(err(`${e.message}；再次启动会继续确认同一执行`)); }
    finally { setBusy(false); }
  };
  return <PageShell page="team">
    <Space style={{ marginBottom: 12 }}><Button type="primary" onClick={() => edit(null)}>创建团队</Button><Button onClick={load}>刷新</Button></Space>
    <Table scroll={{ x: 'max-content' }} rowKey="name" dataSource={rows} columns={[
      { title: '团队', dataIndex: 'name' },
      { title: '成员与职责', render: (_, row) => row.members.map((m) => <div key={m.id}><Tag>{m.agent || m.name}</Tag>{m.role}{m.node_id && ` · ${m.online ? '在线' : '离线'}`}</div>) },
      { title: '队长', dataIndex: 'captain' },
      { title: '操作', render: (_, row) => <Space><Button onClick={() => edit(row)}>编辑</Button><Button onClick={() => { runForm.resetFields(); setLaunch(row); }}>启动团队</Button></Space> },
    ]} />
    <Modal open={editing} onCancel={() => { if (!busy) setEditing(false); }} title="团队成员与职责" footer={null} width={900}>
      <Form form={form} disabled={busy} onFinish={save} layout="vertical">
        <Space><Form.Item name="name" label="团队名称" rules={[{ required: true }, { pattern: /^[a-z0-9][a-z0-9-]{0,63}$/, message: '使用小写字母、数字和连字符' }]}><Input /></Form.Item><Form.Item name="captain" label="队长的成员 ID" rules={[{ required: true }]}><Input /></Form.Item></Space>
        <Form.List name="members">{(fields, { add, remove }) => <>
          {fields.map(({ key, name, ...rest }) => <Space key={key} align="baseline" wrap>
            <Form.Item {...rest} name={[name, 'id']} rules={[{ required: true }]}><Input placeholder="成员 ID" /></Form.Item>
            <Form.Item {...rest} name={[name, 'agent']} rules={[{ required: true }]}><Select style={{ width: 150 }} options={agents} showSearch placeholder="Agent" /></Form.Item>
            <Form.Item {...rest} name={[name, 'role']} rules={[{ required: true }]}><Input style={{ width: 350 }} placeholder="成员职责" /></Form.Item>
            <Button onClick={() => remove(name)}>移除</Button>
          </Space>)}
          <Button onClick={() => add({ agent: 'act' })}>添加成员</Button>
        </>}</Form.List>
        <Button type="primary" htmlType="submit" loading={busy} style={{ marginTop: 16 }}>保存团队</Button>
      </Form>
    </Modal>
    <Modal open={!!launch} title={`启动 ${launch?.name || ''}`} onCancel={() => setLaunch(null)} footer={null}>
      <Form form={runForm} onFinish={run} layout="vertical" initialValues={{ node: '' }}>
        <Alert type="info" showIcon title="整个团队会在同一个执行节点内完成，成员不会跨节点运行" style={{ marginBottom: 12 }} />
        <Form.Item name="node" label="执行节点"><Select options={nodeOptions(nodes, 'team')} /></Form.Item>
        <Form.Item name="prompt" label="任务要求" rules={[{ required: true }]}><Input.TextArea rows={5} /></Form.Item>
        <Button type="primary" htmlType="submit" loading={busy}>启动</Button>
      </Form>
    </Modal>
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </PageShell>;
}
