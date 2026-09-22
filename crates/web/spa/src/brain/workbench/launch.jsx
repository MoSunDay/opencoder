import { Alert, Button, Form, Select, Space, Spin, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../../api.js';
import { useNodes } from '../../fleet/useNodes.js';
import { explicitNodeOptions, newId } from '../../fleet/model.js';
import { EngineeringFields } from './scheduler/fields.jsx';
import { inputRows, launchBody } from './scheduler/model.js';
import { PlanPreview } from './scheduler/editor.jsx';

export function Launch({ onCreated, initialPlan }) {
  const { nodes, error: nodeError } = useNodes(); const [form] = Form.useForm();
  const [plan, setPlan] = useState(null); const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const attempt = useRef(null);
  const [loading, setLoading] = useState(!!initialPlan);
  useEffect(() => {
    if (!initialPlan) return;
    let alive = true; const split = initialPlan.lastIndexOf('@');
    apiGet(`/api/brain/plan-defs/${encodeURIComponent(initialPlan.slice(0, split))}/versions/${initialPlan.slice(split + 1)}`).then((value) => {
      if (!alive) return;
      if (value.plan.schema_version !== 4) throw new Error('不支持的计划版本，请创建分层计划');
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
      if (receipt.schema_version !== 4 || receipt.run_id !== attempt.current.id) throw new Error('未收到匹配的 运行回执');
      attempt.current = null; onCreated(receipt.run_id);
    } catch (error) { setError(error.message); } finally { setBusy(false); }
  };
  return <div className="brain-launch"><Typography.Title level={4}>{plan?.plan.title || '开始大脑调度'}</Typography.Title>
    {(error || nodeError) && <Alert type="error" showIcon title={error || nodeError} />}
    {loading ? <Spin /> : <Form form={form} layout="vertical" onFinish={submit} initialValues={{ engineering: [] }} disabled={busy}>
      {plan && <><Typography.Paragraph>{plan.plan.objective}</Typography.Paragraph><PlanPreview plan={plan.plan} /><Typography.Paragraph type="secondary">执行计划 v{plan.version}；本次工程输入可以调整。</Typography.Paragraph></>}
      <Form.Item name="node" label="大脑所在节点" rules={[{ required: true, message: '请选择节点' }]}><Select options={explicitNodeOptions(nodes, 'brain')} placeholder="选择执行节点" /></Form.Item>
      <EngineeringFields />
      <Space><Button type="primary" htmlType="submit" loading={busy} disabled={!!initialPlan && !plan}>开始执行</Button></Space>
    </Form>}
  </div>;
}
