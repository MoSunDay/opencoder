import { Alert, Button, Card, Form, Input, Select, Space, Tabs } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiPost, apiPut } from '../api.js';
import { BrainPanel } from '../brainPanel.jsx';
import { ExecutionDetail } from './detail.jsx';
import { KINDS, newId, nodeOptions } from './model.js';
import { PageShell } from '../shell/pageShell.jsx';
import { err, ok } from '../notice.js';

function Dispatch({ onNotice }) {
  const [capabilities, setCapabilities] = useState([]); const [nodes, setNodes] = useState([]);
  const [preview, setPreview] = useState(null); const [detail, setDetail] = useState(null); const [busy, setBusy] = useState(false);
  const [dispatchError, setDispatchError] = useState('');
  const [kind, setKind] = useState('agent'); const attempt = useRef(null); const [binding] = Form.useForm(); const [form] = Form.useForm();
  const load = async () => {
    try { const [a, b] = await Promise.all([apiGet('/api/brain/capabilities'), apiGet('/api/nodes')]); setCapabilities(a.capabilities || []); setNodes(b.nodes); }
    catch (e) { onNotice(err(e.message)); }
  };
  useEffect(() => { load(); }, []);
  const bind = async (values) => {
    try { await apiPut(`/api/brain/capabilities/${encodeURIComponent(values.capability)}/target`, { kind, target: values.target }); onNotice(ok('能力已绑定执行目标')); }
    catch (e) { onNotice(err(e.message)); }
  };
  const choose = async (id) => {
    try { const j = await apiGet(`/api/brain/capabilities/${encodeURIComponent(id)}/target`); setKind(j.target?.kind || 'agent'); binding.setFieldValue('target', j.target?.target || ''); }
    catch (e) { onNotice(err(e.message)); }
  };
  const dispatch = async (execute) => {
    let values;
    try { values = await form.validateFields(); } catch { return; }
    const body = { situation: values.situation, node_id: values.node || null };
    const signature = JSON.stringify(body);
    if (attempt.current?.signature !== signature) attempt.current = { signature, requestId: newId('request') };
    setBusy(true);
    try {
      const result = await apiPost(execute ? '/api/brain/dispatch' : '/api/brain/preview', { ...body, request_id: attempt.current.requestId });
      setDispatchError(''); onNotice(err('')); setPreview(result); if (result.execution) { setDetail(result.execution); attempt.current = null; }
    } catch (e) { setDispatchError(e.message); onNotice(err(e.message)); }
    finally { setBusy(false); }
  };
  return <Space orientation="vertical" style={{ width: '100%' }}>
    <Card title="绑定能力执行目标" extra={<Button onClick={load}>刷新能力</Button>}>
      <Form form={binding} layout="vertical" onFinish={bind}>
        <Form.Item name="capability" label="能力" rules={[{ required: true }]}><Select onChange={choose} options={capabilities.map((entry) => { const c = entry.capability || entry; return { value: c.id, label: c.summary }; })} /></Form.Item>
        <Space wrap><Select value={kind} onChange={setKind} options={KINDS.filter((k) => ['agent', 'team', 'dag', 'todos'].includes(k.value))} style={{ width: 160 }} /><Form.Item name="target" rules={[{ required: true }]} style={{ marginBottom: 0 }}><Input style={{ width: 350, maxWidth: '100%' }} placeholder="Agent / 团队 / DAG 名称 / 模板名/v1" /></Form.Item><Button htmlType="submit">保存绑定</Button></Space>
      </Form>
    </Card>
    <Card title="大脑调度">
      <Form form={form} layout="vertical" initialValues={{ node: '' }}>
        <Form.Item name="situation" label="任务目标" rules={[{ required: true }]}><Input.TextArea rows={4} /></Form.Item>
        <Form.Item name="node" label="调度节点"><Select options={nodeOptions(nodes, null)} /></Form.Item>
        <Space><Button loading={busy} onClick={() => dispatch(false)}>预览路由</Button><Button type="primary" loading={busy} onClick={() => dispatch(true)}>直接调度执行</Button></Space>
      </Form>
      {dispatchError && <Alert style={{ marginTop: 12 }} type="error" showIcon title="调度尚未确认" description={`${dispatchError}。保持任务目标不变再次提交，会继续确认同一次执行。`} />}
      {preview && <pre style={{ whiteSpace: 'pre-wrap', marginTop: 16 }}>{JSON.stringify(preview, null, 2)}</pre>}
    </Card>
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </Space>;
}
export function FleetBrainPanel({ onNotice }) {
  return <PageShell page="brain">
    <Tabs items={[{ key: 'dispatch', label: '调度与绑定', children: <Dispatch onNotice={onNotice} /> }, { key: 'library', label: '能力库', children: <BrainPanel /> }]} />
  </PageShell>;
}
