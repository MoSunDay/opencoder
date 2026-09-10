import { Alert, Button, Form, Input, Modal, Select, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { ExecutionDetail } from './detail.jsx';
import { tableLoading, tableRows } from '../ui/tableLoading.js';
import { newId, nodeOptions } from './model.js';
import { err } from '../notice.js';

/// 实时成员名单：captain 永远置顶，其余按已选顺序排列，整体按 agent 去重。
const rosterOf = (captain, members) => [...new Set([captain, ...(members || [])].filter(Boolean))];

export function FleetTeamsPanel({ onNotice }) {
  const [rows, setRows] = useState([]); const [nodes, setNodes] = useState([]); const [agents, setAgents] = useState([]);
  const [editing, setEditing] = useState(false); const [launch, setLaunch] = useState(null); const [detail, setDetail] = useState(null);
  const [busy, setBusy] = useState(false); const [form] = Form.useForm(); const [runForm] = Form.useForm(); const attempt = useRef(null);
  /// 团队列表拉取态（首屏 + 保存/启动后的刷新），驱动表格 loading。
  const [loading, setLoading] = useState(true);
  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [a, b, c] = await Promise.all([apiGet('/api/teams'), apiGet('/api/nodes'), apiGet('/api/brain/agents')]);
      setRows(a.teams); setNodes(b.nodes); setAgents(c.agents || []);
    } catch (e) { onNotice(err(e.message)); }
    finally { setLoading(false); }
  }, [onNotice]);
  useEffect(() => { load(); }, [load]);
  const edit = (row) => {
    form.resetFields();
    if (row) form.setFieldsValue({ name: row.name, captain: row.captain, members: (row.members || []).map((m) => m.agent) });
    setEditing(true);
  };
  const save = async (values) => {
    /// 成员身份即 agent：captain 自动并入成员并置顶去重，职责由服务端固化。
    const payload = { name: values.name, captain: values.captain, members: [...new Set([values.captain, ...(values.members || [])])].map((agent) => ({ agent })) };
    setBusy(true);
    try { await apiPost('/api/teams', payload); onNotice(err('')); setEditing(false); await load(); }
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
  /// 编辑表单实时名单：跟随 captain/队员选择联动，能力画像取自 /api/brain/agents。
  const captain = Form.useWatch('captain', form); const members = Form.useWatch('members', form);
  const roster = rosterOf(captain, members);
  const summaryOf = (agent) => (agents.find((a) => a.agent === agent)?.capabilities || []).map((c) => c.summary).join('；');
  const options = agents.map((a) => ({ value: a.agent, label: a.agent }));
  return <PageShell page="team">
    <Space style={{ marginBottom: 12 }}><Button type="primary" onClick={() => edit(null)}>创建团队</Button><Button onClick={() => load()}>刷新</Button></Space>
    <Table scroll={{ x: 'max-content' }} rowKey="name" dataSource={tableRows(loading, rows)} loading={tableLoading(loading)} columns={[
      { title: '团队', dataIndex: 'name' },
      { title: '成员', render: (_, row) => row.members.map((m) => <Tag key={m.agent}>{m.agent}</Tag>) },
      { title: '队长', dataIndex: 'captain' },
      { title: '操作', render: (_, row) => <Space><Button onClick={() => edit(row)}>编辑</Button><Button onClick={() => { runForm.resetFields(); setLaunch(row); }}>启动团队</Button></Space> },
    ]} />
    <Modal open={editing} onCancel={() => { if (!busy) setEditing(false); }} title="团队成员" footer={null} width={720}>
      <Form form={form} disabled={busy} onFinish={save} layout="vertical">
        <Space wrap>
          <Form.Item name="name" label="团队名称" rules={[{ required: true }, { pattern: /^[a-z0-9][a-z0-9-]{0,63}$/, message: '使用小写字母、数字和连字符' }]}><Input /></Form.Item>
          <Form.Item name="captain" label="队长" rules={[{ required: true }]}><Select showSearch optionFilterProp="label" options={options} style={{ width: 200 }} placeholder="选择队长" /></Form.Item>
        </Space>
        <Form.Item name="members" label="队员"><Select mode="multiple" showSearch optionFilterProp="label" options={options} placeholder="选择团队成员" /></Form.Item>
        {roster.length > 0 && <div style={{ marginBottom: 16 }}>
          {roster.map((agent) => <div key={agent} data-agent={agent} style={{ marginBottom: 4 }}>
            <Tag>{agent}</Tag>{agent === captain && <Tag color="gold">队长</Tag>}
            <span>{summaryOf(agent) || '暂无能力画像'}</span>
          </div>)}
        </div>}
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
