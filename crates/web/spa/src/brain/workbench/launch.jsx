import { Alert, Collapse, Button, Form, Select, Space, Spin, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from '../../api.js';
import { useNodes } from '../../fleet/useNodes.js';
import { explicitNodeOptions, newId } from '../../fleet/model.js';
import { EngineeringFields } from './scheduler/fields.jsx';
import { inputRows, launchBody } from './scheduler/model.js';
import { PlanPreview } from './scheduler/editor.jsx';
import { ProblemFields, isProblemPlan } from './problem/fields.jsx';

export function Launch({ onCreated, initialPlan, capabilities }) {
  const { nodes, error: nodeError } = useNodes(); const [form] = Form.useForm();
  const [plan, setPlan] = useState(null); const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const attempt = useRef(null);
  const [loading, setLoading] = useState(!!initialPlan);
  const [uploading, setUploading] = useState(false);
  useEffect(() => {
    if (!initialPlan) return;
    let alive = true; const split = initialPlan.lastIndexOf('@');
    apiGet(`/api/brain/plan-defs/${encodeURIComponent(initialPlan.slice(0, split))}/versions/${initialPlan.slice(split + 1)}`).then((value) => {
      if (!alive) return;
      if (value.plan.schema_version !== 7) throw new Error('历史计划只读，请创建新版里程碑计划');
      setPlan(value); form.setFieldsValue({ engineering: isProblemPlan(value) ? [] : inputRows(value.plan.inputs), settings: value.plan.inputs?.settings, problemText: value.plan.inputs?.problem?.text || '', problemImages: value.plan.inputs?.problem?.images || [] });
    }).catch((error) => { if (alive) setError(error.message); }).finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [initialPlan, form]);
  const submit = async (values) => {
    if (busy || uploading || (initialPlan && !plan)) return; setBusy(true); setError('');
    try {
      const body = launchBody(values, '', plan); const signature = JSON.stringify(body);
      if (attempt.current?.signature !== signature) attempt.current = { signature, id: newId('brain') };
      const receipt = await apiPost('/api/brain/runs', { ...body, id: attempt.current.id });
      if (receipt.schema_version !== 7 || receipt.run_id !== attempt.current.id) throw new Error('未收到匹配的运行回执');
      attempt.current = null; onCreated(receipt.run_id);
    } catch (error) { setError(error.message); } finally { setBusy(false); }
  };
  return <div className="brain-launch"><Typography.Title level={4}>{plan?.plan.title || '开始大脑调度'}</Typography.Title>
    {(error || nodeError) && <Alert type="error" showIcon title={error || nodeError} />}
    {loading ? <Spin /> : <Form form={form} layout="vertical" onFinish={submit} initialValues={{ engineering: [] }} disabled={busy}>
      {plan && (isProblemPlan(plan) ? <><Typography.Paragraph>描述问题并添加截图，定位影响范围，在 Windows 客户端复现和收集证据；需要时构建修改包复测。</Typography.Paragraph><Collapse items={[{ key: 'flow', label: '查看诊断流程', children: <PlanPreview plan={plan.plan} capabilities={capabilities} /> }]} /><Typography.Paragraph type="secondary">问题描述和图片会保留在本次运行中。</Typography.Paragraph></> : <><Typography.Paragraph>{plan.plan.objective}</Typography.Paragraph><PlanPreview plan={plan.plan} capabilities={capabilities} /><Typography.Paragraph type="secondary">执行计划 v{plan.version}；本次工程输入可以调整。</Typography.Paragraph></>)}
      <Form.Item name="node" label="大脑所在节点" rules={[{ required: true, message: '请选择节点' }]}><Select options={explicitNodeOptions(nodes, 'brain')} placeholder="选择执行节点" /></Form.Item>
      {isProblemPlan(plan) ? <ProblemFields onBusy={setUploading} nodes={nodes} /> : <EngineeringFields />}
      <Space><Button type="primary" htmlType="submit" loading={busy} disabled={uploading || (!!initialPlan && !plan)}>开始执行</Button></Space>
    </Form>}
  </div>;
}
