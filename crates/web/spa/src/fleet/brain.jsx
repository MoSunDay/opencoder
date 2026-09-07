import { Alert, Button, Form, Input, Select, Space, Tabs, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiPost } from '../api.js';
import { BrainPanel } from '../brainPanel.jsx';
import { ExecutionDetail } from './detail.jsx';
import { useNodes } from './useNodes.js';
import { canUseNode, explicitNodeOptions, newId } from './model.js';
import { PageShell } from '../shell/pageShell.jsx';
import { err } from '../notice.js';

export function BrainDispatch({ onNotice }) {
  const { nodes, error: nodesError } = useNodes();
  const [detail, setDetail] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const attempt = useRef(null);
  const inFlight = useRef(false);
  const [form] = Form.useForm();

  const dispatch = async (values) => {
    if (inFlight.current) return;
    if (!canUseNode(nodes, values.node)) { setError('请选择可执行的目标节点'); return; }
    const body = { situation: values.situation.trim(), node_id: values.node };
    const signature = JSON.stringify(body);
    if (attempt.current?.signature !== signature) attempt.current = { signature, requestId: newId('request') };
    inFlight.current = true; setBusy(true); setError('');
    try {
      const result = await apiPost('/api/brain/dispatch', { ...body, request_id: attempt.current.requestId });
      if (!result.execution?.id) throw new Error('服务未返回执行记录，请重试确认');
      onNotice?.(err('')); setDetail(result.execution); attempt.current = null;
    } catch (e) { setError(e.message); onNotice?.(err(e.message)); }
    finally { inFlight.current = false; setBusy(false); }
  };

  return <>
    {nodesError && <Alert type="error" showIcon title={nodesError} style={{ marginBottom: 16 }} />}
    <Typography.Paragraph type="secondary">选择目标节点并描述需求，提交后开始执行。</Typography.Paragraph>
    <Form form={form} layout="vertical" onFinish={dispatch} disabled={busy} style={{ maxWidth: 960 }}>
      <Form.Item name="node" label="目标节点" rules={[{ required: true, message: '请选择目标节点' }]}>
        <Select placeholder="请选择目标节点" options={explicitNodeOptions(nodes)} showSearch optionFilterProp="label" />
      </Form.Item>
      <Form.Item name="situation" label="需求" rules={[{ required: true, whitespace: true, message: '请输入需求' }]}>
        <Input.TextArea rows={6} placeholder="描述你希望完成的任务" />
      </Form.Item>
      <Space><Button type="primary" htmlType="submit" loading={busy}>开始执行</Button></Space>
    </Form>
    {error && <Alert style={{ marginTop: 16 }} type="error" showIcon title="执行未确认" description={error} />}
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </>;
}

export function FleetBrainPanel({ onNotice }) {
  return <PageShell page="brain"><Tabs items={[
    { key: 'dispatch', label: '需求执行', children: <BrainDispatch onNotice={onNotice} /> },
    { key: 'library', label: '能力库', children: <BrainPanel onNotice={onNotice} /> },
  ]} /></PageShell>;
}
