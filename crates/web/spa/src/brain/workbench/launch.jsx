import { Alert, Button, Form, Input, Select, Space, Spin, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../../api.js';
import { useNodes } from '../../fleet/useNodes.js';
import { explicitNodeOptions, newId } from '../../fleet/model.js';
import { CapabilityPicker, EngineeringFields } from './scheduler/fields.jsx';
import { inputRows, launchBody } from './scheduler/model.js';
import { SummaryCanvas } from './summaryCanvas.jsx';

export function Launch({ capabilities = [], onCreated, initialPlan }) {
  const { nodes, error: nodeError } = useNodes(); const [form] = Form.useForm();
  const [plan, setPlan] = useState(null); const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const attempt = useRef(null);
  const [loading, setLoading] = useState(!!initialPlan);
  useEffect(() => {
    if (!initialPlan) return;
    let alive = true; const split = initialPlan.lastIndexOf('@');
    apiGet(`/api/brain/plan-defs/${encodeURIComponent(initialPlan.slice(0, split))}/versions/${initialPlan.slice(split + 1)}`).then((value) => {
      if (!alive) return;
      if (value.plan.schema_version !== 3) throw new Error('历史计划只读，请创建新的调度计划');
      setPlan(value); form.setFieldsValue({ engineering: inputRows(value.plan.inputs) });
    }).catch((error) => { if (alive) setError(error.message); }).finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [initialPlan, form]);
  const submit = async (values) => {
    if (busy || (initialPlan && !plan)) return; setBusy(true); setError('');
    try {
      const body = launchBody(values, '', plan); const signature = JSON.stringify(body);
      if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('brain') };
      const receipt = await apiPost('/api/brain/runs', { ...body, id: attempt.current.id });
      if (!receipt.run_id) throw new Error('未收到运行回执');
      attempt.current = null; onCreated(receipt.run_id);
    } catch (error) { setError(error.message); } finally { setBusy(false); }
  };
  return <div className="brain-launch"><Typography.Title level={4}>{plan?.plan.title || '开始大脑调度'}</Typography.Title>
    {(error || nodeError) && <Alert type="error" showIcon title={error || nodeError} />}
    {loading ? <Spin /> : <Form form={form} layout="vertical" onFinish={submit} initialValues={{ engineering: [], capability_ids: [] }} disabled={busy}>
      {plan ? <><Typography.Paragraph>{plan.plan.objective}</Typography.Paragraph><SummaryCanvas view={{ ...plan.plan, capabilities: capabilities.filter((cap) => plan.plan.capability_ids.includes(cap.id)) }} /><Typography.Paragraph type="secondary">使用计划 v{plan.version} 的能力范围；本次工程输入可以调整。</Typography.Paragraph></>
        : <><Form.Item name="objective" label="目标和交付物" rules={[{ required: true, whitespace: true, message: '请输入目标和交付物' }]}><Input.TextArea rows={3} /></Form.Item>
          <Form.Item name="capability_ids" label="关联能力库" rules={[{ type: 'array', min: 1, required: true, message: '请至少选择一项能力' }]}><CapabilityPicker capabilities={capabilities} /></Form.Item></>}
      <Form.Item name="node" label="大脑所在节点" rules={[{ required: true, message: '请选择节点' }]}><Select options={explicitNodeOptions(nodes, 'brain')} placeholder="选择执行节点" /></Form.Item>
      <EngineeringFields />
      <Space><Button type="primary" htmlType="submit" loading={busy} disabled={!!initialPlan && !plan}>开始执行</Button></Space>
    </Form>}
  </div>;
}
